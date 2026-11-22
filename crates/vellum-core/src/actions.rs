//! app level commands: everything in the menus, the command palette and the shortcut
//! editor that is not an edit to one document.
//!
//! they are ordinary [`Command`]s on an [`AppContext`], registered in one
//! [`CommandRegistry`], so the palette, shortcuts, macros and the cli all see the same list.
//! none of them are undoable; undo belongs to documents.
//!
//! two flavours:
//! - **rust actions** change something rust owns (settings, documents, jobs) right here.
//! - **ui actions** only make sense in the window, like opening the palette. running one
//!   adds its record to the outcome and the ui performs it. they still go through rust so
//!   a recorded macro or the cli sees every step the user took.

use serde::{Deserialize, Serialize};

use crate::app::{AppServices, CommandOutcome, SaveOutcome};
use crate::command::{
    Command, CommandError, CommandInfo, CommandRecord, CommandRegistry, params_of,
};
use crate::recent;
use crate::settings::Theme;

/// what an app command can reach while it runs
pub struct AppContext {
    pub services: AppServices,
    /// the document the command is about, usually the active tab
    pub document: Option<String>,
    pub outcome: CommandOutcome,
}

impl AppContext {
    fn note(&mut self, message: impl Into<String>) {
        self.outcome.message = Some(message.into());
    }

    fn update_settings(
        &self,
        f: impl FnOnce(&mut crate::settings::AppSettings),
    ) -> Result<(), CommandError> {
        self.services
            .settings
            .update(f)
            .map(|_| ())
            .map_err(CommandError::failed)
    }
}

fn not_undoable() -> Result<(), CommandError> {
    Err(CommandError::failed("app commands cannot be undone"))
}

/// declares commands with no parameters. each gets a struct, its palette entry and the
/// code that runs it.
macro_rules! app_commands {
    ($(
        $name:ident = $kind:literal, $category:literal, $title:literal, $description:literal,
        |$ctx:ident| $body:block
    )*) => {
        $(
            #[derive(Debug, Default, Serialize, Deserialize)]
            pub struct $name {}

            impl Command<AppContext> for $name {
                fn kind(&self) -> &'static str {
                    $kind
                }
                fn name(&self) -> String {
                    $title.to_owned()
                }
                fn apply(&mut self, $ctx: &mut AppContext) -> Result<(), CommandError> {
                    $body
                }
                fn revert(&mut self, _: &mut AppContext) -> Result<(), CommandError> {
                    not_undoable()
                }
                fn params(&self) -> Result<serde_json::Value, CommandError> {
                    params_of(self)
                }
                fn undoable(&self) -> bool {
                    false
                }
            }
        )*

        fn register_all(registry: &mut CommandRegistry<AppContext>) -> Result<(), CommandError> {
            $(
                registry.register::<$name>(CommandInfo {
                    kind: $kind,
                    title: $title,
                    category: $category,
                    description: $description,
                })?;
            )*
            Ok(())
        }
    };
}

/// hands the command back to the ui to perform
fn ui(ctx: &mut AppContext, kind: &str) -> Result<(), CommandError> {
    ctx.outcome
        .ui_actions
        .push(CommandRecord::new(kind, serde_json::json!({})));
    Ok(())
}

fn active_document(ctx: &AppContext) -> Option<String> {
    ctx.document
        .clone()
        .filter(|id| ctx.services.documents.is_open(id))
}

fn set_theme(ctx: &AppContext, theme: Theme) -> Result<(), CommandError> {
    ctx.update_settings(|s| s.appearance.theme = theme)
}

/// the ui scale steps zoom in and out walk through
pub const ZOOM_STEPS: [f32; 9] = [0.75, 0.85, 0.9, 1.0, 1.1, 1.25, 1.5, 1.75, 2.0];

fn zoom(ctx: &AppContext, direction: i8) -> Result<(), CommandError> {
    ctx.update_settings(|s| {
        let current = s.appearance.ui_scale;
        let next = match direction {
            d if d > 0 => ZOOM_STEPS.iter().find(|step| **step > current + 0.001),
            d if d < 0 => ZOOM_STEPS
                .iter()
                .rev()
                .find(|step| **step < current - 0.001),
            _ => Some(&1.0),
        };
        if let Some(next) = next {
            s.appearance.ui_scale = *next;
        }
    })
}

app_commands! {
    OpenPalette = "app.command_palette", "App", "Command palette",
        "Search every command, setting, recent file and preset.",
        |ctx| { ui(ctx, "app.command_palette") }
    OpenSettings = "app.settings", "App", "Settings",
        "Change themes, autosave, jobs and more.",
        |ctx| { ui(ctx, "app.settings") }
    OpenShortcuts = "app.shortcuts", "App", "Keyboard shortcuts",
        "See and change every shortcut, or pick a preset from another app.",
        |ctx| { ui(ctx, "app.shortcuts") }
    GoHome = "app.home", "App", "Go to home",
        "Show recent files and quick tools.",
        |ctx| { ui(ctx, "app.home") }
    ShowNotifications = "app.notifications", "App", "Notifications",
        "Show finished jobs and other news.",
        |ctx| { ui(ctx, "app.notifications") }

    OpenFile = "file.open", "File", "Open file",
        "Pick files to open in the right workspace.",
        |ctx| { ui(ctx, "file.open") }
    SaveDocument = "file.save", "File", "Save",
        "Save the current document.",
        |ctx| {
            let Some(id) = active_document(ctx) else {
                ctx.note("Open a document to save it");
                return Ok(());
            };
            let saver = ctx
                .services
                .documents
                .doc_type(&id)
                .ok()
                .and_then(|t| ctx.services.saver(t));
            match saver {
                Some(save) => match save(&id).map_err(CommandError::failed)? {
                    SaveOutcome::Started => Ok(()),
                    SaveOutcome::NeedsPath => ui(ctx, "file.save_as"),
                },
                // placeholders have no file format, saving only marks them saved
                None => ctx
                    .services
                    .documents
                    .mark_saved(&id)
                    .map_err(CommandError::failed),
            }
        }
    CloseTab = "file.close", "File", "Close tab",
        "Close the current document. Unsaved changes ask first.",
        |ctx| { ui(ctx, "file.close") }

    Undo = "edit.undo", "Edit", "Undo",
        "Undo the last change in the current document.",
        |ctx| {
            let Some(id) = active_document(ctx) else {
                ctx.note("Nothing to undo");
                return Ok(());
            };
            let undone = ctx.services.documents.undo(&id).map_err(CommandError::failed)?;
            if !undone {
                ctx.note("Nothing to undo");
            }
            Ok(())
        }
    Redo = "edit.redo", "Edit", "Redo",
        "Redo the change you last undid.",
        |ctx| {
            let Some(id) = active_document(ctx) else {
                ctx.note("Nothing to redo");
                return Ok(());
            };
            let redone = ctx.services.documents.redo(&id).map_err(CommandError::failed)?;
            if !redone {
                ctx.note("Nothing to redo");
            }
            Ok(())
        }

    NextTab = "window.next_tab", "Window", "Next tab",
        "Switch to the tab on the right.",
        |ctx| { ui(ctx, "window.next_tab") }
    PreviousTab = "window.previous_tab", "Window", "Previous tab",
        "Switch to the tab on the left.",
        |ctx| { ui(ctx, "window.previous_tab") }
    TearOff = "window.tear_off", "Window", "Move tab to a new window",
        "Open the current document in its own window.",
        |ctx| { ui(ctx, "window.tear_off") }
    MoveToMain = "window.move_to_main", "Window", "Move tab to the main window",
        "Send the current document back to the main window.",
        |ctx| { ui(ctx, "window.move_to_main") }

    ToggleDocks = "view.docks.toggle", "View", "Hide or show panels",
        "Give the document the whole window, or bring the panels back.",
        |ctx| { ui(ctx, "view.docks.toggle") }
    ShowJobs = "view.panel.jobs", "View", "Jobs panel",
        "Show exports, conversions and other running work.",
        |ctx| { ui(ctx, "view.panel.jobs") }
    ShowHistory = "view.panel.history", "View", "History panel",
        "Show every step of the current document, including other branches.",
        |ctx| { ui(ctx, "view.panel.history") }
    ShowInfo = "view.panel.info", "View", "Info panel",
        "Show details about the current document.",
        |ctx| { ui(ctx, "view.panel.info") }
    SaveLayout = "view.layout.save", "View", "Save layout",
        "Keep the current panel arrangement under a name.",
        |ctx| { ui(ctx, "view.layout.save") }
    ResetLayout = "view.layout.reset", "View", "Reset layout",
        "Put every panel back where it started.",
        |ctx| { ui(ctx, "view.layout.reset") }
    DarkTheme = "view.theme.dark", "View", "Dark theme",
        "Use the dark theme.",
        |ctx| { set_theme(ctx, Theme::Dark) }
    LightTheme = "view.theme.light", "View", "Light theme",
        "Use the light theme.",
        |ctx| { set_theme(ctx, Theme::Light) }
    HighContrastTheme = "view.theme.high_contrast", "View", "High contrast theme",
        "Use the high contrast theme.",
        |ctx| { set_theme(ctx, Theme::HighContrast) }
    ToggleCompact = "view.compact.toggle", "View", "Compact mode",
        "Turn tighter rows and padding on or off.",
        |ctx| { ctx.update_settings(|s| s.appearance.compact = !s.appearance.compact) }
    ToggleReducedMotion = "view.reduced_motion.toggle", "View", "Reduce motion",
        "Turn animations on or off.",
        |ctx| { ctx.update_settings(|s| s.appearance.reduced_motion = !s.appearance.reduced_motion) }
    ZoomIn = "view.zoom.in", "View", "Zoom in",
        "Make the interface bigger.",
        |ctx| { zoom(ctx, 1) }
    ZoomOut = "view.zoom.out", "View", "Zoom out",
        "Make the interface smaller.",
        |ctx| { zoom(ctx, -1) }
    ZoomReset = "view.zoom.reset", "View", "Actual size",
        "Set the interface back to 100%.",
        |ctx| { zoom(ctx, 0) }

    QuickToolsWorkspace = "workspace.quick_tools", "Workspace", "Quick tools",
        "Go to the quick tools workspace.",
        |ctx| { ui(ctx, "workspace.quick_tools") }
    ImageWorkspace = "workspace.image", "Workspace", "Image workspace",
        "Go to the image editor.",
        |ctx| { ui(ctx, "workspace.image") }
    VideoWorkspace = "workspace.video", "Workspace", "Video workspace",
        "Go to the video editor.",
        |ctx| { ui(ctx, "workspace.video") }
    ColorWorkspace = "workspace.color", "Workspace", "Colour workspace",
        "Go to colour grading.",
        |ctx| { ui(ctx, "workspace.color") }
    AudioWorkspace = "workspace.audio", "Workspace", "Audio workspace",
        "Go to the audio editor.",
        |ctx| { ui(ctx, "workspace.audio") }
    MotionWorkspace = "workspace.motion", "Workspace", "Motion workspace",
        "Go to motion graphics.",
        |ctx| { ui(ctx, "workspace.motion") }
    VectorWorkspace = "workspace.vector", "Workspace", "Vector workspace",
        "Go to vector and PDF tools.",
        |ctx| { ui(ctx, "workspace.vector") }

    ExportToSize = "tool.export_to_size", "Quick tools", "Export to 10 MB",
        "Shrink a video, GIF, image or song to fit a size limit.",
        |ctx| { ui(ctx, "tool.export_to_size") }
    VideoToGif = "tool.video_to_gif", "Quick tools", "Video to GIF",
        "Turn part of a video into a GIF.",
        |ctx| { ui(ctx, "tool.video_to_gif") }
    GifTools = "tool.gif_tools", "Quick tools", "GIF tools",
        "Edit a GIF frame by frame, make it smaller, split it into pictures or turn it into a video.",
        |ctx| { ui(ctx, "tool.gif_tools") }
    RemoveBackground = "tool.remove_background", "Quick tools", "Remove background",
        "Cut the subject out of a photo.",
        |ctx| { ui(ctx, "tool.remove_background") }
    Convert = "tool.convert", "Quick tools", "Convert",
        "Change a file to another format.",
        |ctx| { ui(ctx, "tool.convert") }
    ScreenRecord = "tool.screen_record", "Quick tools", "Screen record",
        "Record your screen, a window or an area.",
        |ctx| { ui(ctx, "tool.screen_record") }

    NewImage = "file.new", "File", "New image",
        "Make a new blank picture.",
        |ctx| { ui(ctx, "file.new") }
    SaveAs = "file.save_as", "File", "Save as",
        "Save the current document under a new name.",
        |ctx| { ui(ctx, "file.save_as") }
    ImageExport = "image.export", "Image", "Export as",
        "Save a copy as PNG, JPEG, WebP or AVIF, optionally under a size limit.",
        |ctx| { ui(ctx, "image.export") }
    ToolMove = "image.tool.move", "Image tools", "Move tool",
        "Move layers and selections.",
        |ctx| { ui(ctx, "image.tool.move") }
    ToolMarquee = "image.tool.marquee", "Image tools", "Marquee tool",
        "Select a rectangle or an ellipse.",
        |ctx| { ui(ctx, "image.tool.marquee") }
    ToolMarqueeCycle = "image.tool.marquee_cycle", "Image tools", "Next marquee tool",
        "Switch between the rectangle and ellipse marquee.",
        |ctx| { ui(ctx, "image.tool.marquee_cycle") }
    ToolLasso = "image.tool.lasso", "Image tools", "Lasso tool",
        "Select by drawing around something.",
        |ctx| { ui(ctx, "image.tool.lasso") }
    ToolLassoCycle = "image.tool.lasso_cycle", "Image tools", "Next lasso tool",
        "Switch between the lasso, polygon lasso and magnetic lasso.",
        |ctx| { ui(ctx, "image.tool.lasso_cycle") }
    ToolWand = "image.tool.wand", "Image tools", "Magic wand tool",
        "Select areas of similar colour with a click.",
        |ctx| { ui(ctx, "image.tool.wand") }
    ToolWandCycle = "image.tool.wand_cycle", "Image tools", "Next wand tool",
        "Switch between the magic wand and the quick selection brush.",
        |ctx| { ui(ctx, "image.tool.wand_cycle") }
    ToolCrop = "image.tool.crop", "Image tools", "Crop tool",
        "Cut the picture down, straighten it or fix its perspective.",
        |ctx| { ui(ctx, "image.tool.crop") }
    ToolCropCycle = "image.tool.crop_cycle", "Image tools", "Next crop tool",
        "Switch between crop and perspective crop.",
        |ctx| { ui(ctx, "image.tool.crop_cycle") }
    ToolEyedropper = "image.tool.eyedropper", "Image tools", "Eyedropper tool",
        "Pick a colour from the picture.",
        |ctx| { ui(ctx, "image.tool.eyedropper") }
    ToolBrush = "image.tool.brush", "Image tools", "Brush tool",
        "Paint with a soft or hard brush.",
        |ctx| { ui(ctx, "image.tool.brush") }
    ToolBrushCycle = "image.tool.brush_cycle", "Image tools", "Next brush tool",
        "Switch between the brush and the pencil.",
        |ctx| { ui(ctx, "image.tool.brush_cycle") }
    ToolEraser = "image.tool.eraser", "Image tools", "Eraser tool",
        "Rub out pixels.",
        |ctx| { ui(ctx, "image.tool.eraser") }
    ToolGradient = "image.tool.gradient", "Image tools", "Gradient tool",
        "Fill with a gradient.",
        |ctx| { ui(ctx, "image.tool.gradient") }
    ToolGradientCycle = "image.tool.gradient_cycle", "Image tools", "Next fill tool",
        "Switch between the gradient and the paint bucket.",
        |ctx| { ui(ctx, "image.tool.gradient_cycle") }
    ToolHand = "image.tool.hand", "Image tools", "Hand tool",
        "Drag to move around the picture.",
        |ctx| { ui(ctx, "image.tool.hand") }
    ToolZoom = "image.tool.zoom", "Image tools", "Zoom tool",
        "Click to zoom in, Alt click to zoom out.",
        |ctx| { ui(ctx, "image.tool.zoom") }
    FreeTransform = "image.transform", "Image", "Free transform",
        "Scale, rotate, skew, distort or warp the layer or selection.",
        |ctx| { ui(ctx, "image.transform") }
    SelectAll = "image.select.all", "Select", "Select all",
        "Select the whole canvas.",
        |ctx| { ui(ctx, "image.select.all") }
    Deselect = "image.select.deselect", "Select", "Deselect",
        "Remove the selection.",
        |ctx| { ui(ctx, "image.select.deselect") }
    Reselect = "image.select.reselect", "Select", "Reselect",
        "Bring back the last selection.",
        |ctx| { ui(ctx, "image.select.reselect") }
    InvertSelection = "image.select.invert", "Select", "Invert selection",
        "Select everything that was not selected.",
        |ctx| { ui(ctx, "image.select.invert") }
    FeatherSelection = "image.select.feather", "Select", "Feather selection",
        "Soften the edge of the selection.",
        |ctx| { ui(ctx, "image.select.feather") }
    GrowSelection = "image.select.expand", "Select", "Expand selection",
        "Make the selection bigger by some pixels.",
        |ctx| { ui(ctx, "image.select.expand") }
    ShrinkSelection = "image.select.contract", "Select", "Contract selection",
        "Make the selection smaller by some pixels.",
        |ctx| { ui(ctx, "image.select.contract") }
    BorderSelection = "image.select.border", "Select", "Border selection",
        "Select a band along the edge of the selection.",
        |ctx| { ui(ctx, "image.select.border") }
    SmoothSelection = "image.select.smooth", "Select", "Smooth selection",
        "Round off jagged corners of the selection.",
        |ctx| { ui(ctx, "image.select.smooth") }
    ColorRange = "image.select.color_range", "Select", "Colour range",
        "Select every pixel close to a colour.",
        |ctx| { ui(ctx, "image.select.color_range") }
    TransformSelection = "image.select.transform", "Select", "Transform selection",
        "Scale, rotate or distort the selection outline itself.",
        |ctx| { ui(ctx, "image.select.transform") }
    SaveSelection = "image.select.save", "Select", "Save selection",
        "Keep the selection as a channel to load later.",
        |ctx| { ui(ctx, "image.select.save") }
    LoadSelection = "image.select.load", "Select", "Load selection",
        "Bring back a saved selection.",
        |ctx| { ui(ctx, "image.select.load") }
    QuickMask = "image.quick_mask", "Select", "Quick mask",
        "Paint the selection with brushes, shown as a red overlay.",
        |ctx| { ui(ctx, "image.quick_mask") }
    DefaultColors = "image.colors.default", "Image", "Default colours",
        "Set the colours back to black and white.",
        |ctx| { ui(ctx, "image.colors.default") }
    SwapColors = "image.colors.swap", "Image", "Swap colours",
        "Swap the foreground and background colours.",
        |ctx| { ui(ctx, "image.colors.swap") }
    BrushSmaller = "image.brush.smaller", "Image", "Smaller brush",
        "Make the brush smaller.",
        |ctx| { ui(ctx, "image.brush.smaller") }
    BrushBigger = "image.brush.bigger", "Image", "Bigger brush",
        "Make the brush bigger.",
        |ctx| { ui(ctx, "image.brush.bigger") }
    BrushSofter = "image.brush.softer", "Image", "Softer brush",
        "Make the brush edge softer.",
        |ctx| { ui(ctx, "image.brush.softer") }
    BrushHarder = "image.brush.harder", "Image", "Harder brush",
        "Make the brush edge harder.",
        |ctx| { ui(ctx, "image.brush.harder") }
    ClearPixels = "image.edit.clear", "Image", "Delete selected pixels",
        "Clear the selected pixels of the current layer.",
        |ctx| { ui(ctx, "image.edit.clear") }
    FillForeground = "image.edit.fill_foreground", "Image", "Fill with foreground",
        "Fill the selection with the foreground colour.",
        |ctx| { ui(ctx, "image.edit.fill_foreground") }
    FillBackground = "image.edit.fill_background", "Image", "Fill with background",
        "Fill the selection with the background colour.",
        |ctx| { ui(ctx, "image.edit.fill_background") }
    NewLayer = "image.layer.new", "Layers", "New layer",
        "Add an empty layer above the current one.",
        |ctx| { ui(ctx, "image.layer.new") }
    LayerViaCopy = "image.layer.via_copy", "Layers", "New layer via copy",
        "Copy the selected pixels to a new layer.",
        |ctx| { ui(ctx, "image.layer.via_copy") }
    LayerViaCut = "image.layer.via_cut", "Layers", "New layer via cut",
        "Move the selected pixels to a new layer.",
        |ctx| { ui(ctx, "image.layer.via_cut") }
    DuplicateLayer = "image.layer.duplicate", "Layers", "Duplicate layer",
        "Copy the current layer.",
        |ctx| { ui(ctx, "image.layer.duplicate") }
    DeleteLayer = "image.layer.delete", "Layers", "Delete layer",
        "Remove the current layer.",
        |ctx| { ui(ctx, "image.layer.delete") }
    GroupLayers = "image.layer.group", "Layers", "Group layers",
        "Put the current layer in a new group.",
        |ctx| { ui(ctx, "image.layer.group") }
    UngroupLayers = "image.layer.ungroup", "Layers", "Ungroup layers",
        "Take the layers out of the current group.",
        |ctx| { ui(ctx, "image.layer.ungroup") }
    ClippingMask = "image.layer.clip", "Layers", "Clipping mask",
        "Show the layer only where the layer below has pixels.",
        |ctx| { ui(ctx, "image.layer.clip") }
    MergeDown = "image.layer.merge_down", "Layers", "Merge down",
        "Merge the current layer into the one below.",
        |ctx| { ui(ctx, "image.layer.merge_down") }
    MergeVisible = "image.layer.merge_visible", "Layers", "Merge visible",
        "Merge every visible layer into one.",
        |ctx| { ui(ctx, "image.layer.merge_visible") }
    FlattenImage = "image.layer.flatten", "Layers", "Flatten image",
        "Merge every layer into one.",
        |ctx| { ui(ctx, "image.layer.flatten") }
    AddLayerMask = "image.layer.add_mask", "Layers", "Add layer mask",
        "Add a mask to the current layer, from the selection if there is one.",
        |ctx| { ui(ctx, "image.layer.add_mask") }
    ToggleRulers = "image.view.rulers", "View", "Rulers",
        "Show or hide the rulers.",
        |ctx| { ui(ctx, "image.view.rulers") }
    ToggleGrid = "image.view.grid", "View", "Grid",
        "Show or hide the grid.",
        |ctx| { ui(ctx, "image.view.grid") }
    ToggleGuides = "image.view.guides", "View", "Guides",
        "Show or hide guides.",
        |ctx| { ui(ctx, "image.view.guides") }
    ToggleSnap = "image.view.snap", "View", "Snap",
        "Turn snapping to guides, edges and the grid on or off.",
        |ctx| { ui(ctx, "image.view.snap") }
    LockGuides = "image.view.lock_guides", "View", "Lock guides",
        "Stop guides from being moved by accident.",
        |ctx| { ui(ctx, "image.view.lock_guides") }
    ClearGuides = "image.view.clear_guides", "View", "Clear guides",
        "Remove every guide.",
        |ctx| { ui(ctx, "image.view.clear_guides") }
    ViewFit = "image.view.fit", "View", "Fit on screen",
        "Show the whole picture.",
        |ctx| { ui(ctx, "image.view.fit") }
    ViewActualPixels = "image.view.actual_pixels", "View", "Actual pixels",
        "Show the picture at 100%.",
        |ctx| { ui(ctx, "image.view.actual_pixels") }
    ViewZoomIn = "image.view.zoom_in", "View", "Zoom in on the picture",
        "Zoom in on the picture.",
        |ctx| { ui(ctx, "image.view.zoom_in") }
    ViewZoomOut = "image.view.zoom_out", "View", "Zoom out of the picture",
        "Zoom out of the picture.",
        |ctx| { ui(ctx, "image.view.zoom_out") }
    CanvasSize = "image.canvas_size", "Image", "Canvas size",
        "Add or cut away space around the picture.",
        |ctx| { ui(ctx, "image.canvas_size") }
    ImageSize = "image.image_size", "Image", "Image size",
        "Resize the whole picture.",
        |ctx| { ui(ctx, "image.image_size") }
    CropToSelection = "image.crop_to_selection", "Image", "Crop to selection",
        "Cut the canvas down to the selection.",
        |ctx| { ui(ctx, "image.crop_to_selection") }
    RotateCanvasRight = "image.rotate_right", "Image", "Rotate 90° clockwise",
        "Turn the whole picture a quarter turn clockwise.",
        |ctx| { ui(ctx, "image.rotate_right") }
    RotateCanvasLeft = "image.rotate_left", "Image", "Rotate 90° counter clockwise",
        "Turn the whole picture a quarter turn counter clockwise.",
        |ctx| { ui(ctx, "image.rotate_left") }
    RotateCanvas180 = "image.rotate_180", "Image", "Rotate 180°",
        "Turn the whole picture upside down.",
        |ctx| { ui(ctx, "image.rotate_180") }
    FlipCanvasH = "image.flip_canvas_h", "Image", "Flip canvas horizontally",
        "Mirror the whole picture left to right.",
        |ctx| { ui(ctx, "image.flip_canvas_h") }
    FlipCanvasV = "image.flip_canvas_v", "Image", "Flip canvas vertically",
        "Mirror the whole picture top to bottom.",
        |ctx| { ui(ctx, "image.flip_canvas_v") }
    FlipLayerH = "image.flip_layer_h", "Image", "Flip layer horizontally",
        "Mirror the current layer left to right.",
        |ctx| { ui(ctx, "image.flip_layer_h") }
    FlipLayerV = "image.flip_layer_v", "Image", "Flip layer vertically",
        "Mirror the current layer top to bottom.",
        |ctx| { ui(ctx, "image.flip_layer_v") }
    ClearFinishedJobs = "jobs.clear_finished", "Jobs", "Clear finished jobs",
        "Remove done, failed and cancelled jobs from the list.",
        |ctx| {
            ctx.services.jobs.clear_finished();
            Ok(())
        }
    ClearRecent = "recent.clear", "File", "Clear recent files",
        "Forget recent files. Pinned files stay.",
        |ctx| {
            recent::clear_unpinned(&ctx.services.recent)
                .map(|_| ())
                .map_err(CommandError::failed)
        }
    MarkNotificationsRead = "notifications.mark_read", "App", "Mark notifications as read",
        "Clear the unread dot on the bell.",
        |ctx| {
            ctx.services.notifications.mark_all_read();
            Ok(())
        }
    ClearNotifications = "notifications.clear", "App", "Clear notifications",
        "Remove every notification.",
        |ctx| {
            ctx.services.notifications.clear();
            Ok(())
        }
}

/// every app command, ready for the palette
pub fn registry() -> Result<CommandRegistry<AppContext>, CommandError> {
    let mut registry = CommandRegistry::new();
    register_all(&mut registry)?;
    Ok(registry)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::AppPaths;
    use crate::command::CommandRecord;
    use crate::documents::{DocumentMeta, PlaceholderDocument, RenameDocument};
    use crate::files::FileKind;
    use crate::shortcuts::kinds_in_presets;
    use serde_json::json;
    use std::path::PathBuf;

    fn start() -> (tempfile::TempDir, AppServices) {
        let dir = tempfile::tempdir().unwrap();
        let services = AppServices::start(AppPaths {
            config_dir: dir.path().join("config"),
            data_dir: dir.path().join("data"),
        })
        .unwrap();
        (dir, services)
    }

    fn run(services: &AppServices, kind: &str, document: Option<&str>) -> CommandOutcome {
        services
            .run_command(
                &CommandRecord::new(kind, json!({})),
                document.map(str::to_owned),
            )
            .unwrap()
    }

    #[test]
    fn registry_builds_and_every_preset_kind_exists() {
        let registry = registry().unwrap();
        for kind in kinds_in_presets() {
            assert!(
                registry.info(kind).is_some(),
                "{kind} is in a preset but not registered"
            );
        }
        for info in registry.infos() {
            assert!(
                !info.title.contains('\u{2014}'),
                "{} has an em dash",
                info.kind
            );
            assert!(
                info.description.ends_with('.'),
                "{} needs a full stop",
                info.kind
            );
        }
    }

    #[test]
    fn ui_actions_come_back_to_the_ui() {
        let (_dir, services) = start();
        let outcome = run(&services, "app.command_palette", None);
        assert_eq!(
            outcome.ui_actions,
            [CommandRecord::new("app.command_palette", json!({}))]
        );
        assert_eq!(outcome.message, None);
    }

    #[test]
    fn undo_and_redo_act_on_the_given_document() {
        let (_dir, services) = start();
        let id = services
            .documents
            .open(
                PlaceholderDocument::default(),
                DocumentMeta::for_file(PathBuf::from("/m/a.png"), FileKind::Image),
            )
            .unwrap()
            .id;
        services
            .documents
            .execute(
                &id,
                &CommandRecord::new(RenameDocument::KIND, json!({ "title": "Cover" })),
            )
            .unwrap();

        run(&services, "edit.undo", Some(&id));
        assert_eq!(services.documents.info(&id).unwrap().title, "a.png");
        let outcome = run(&services, "edit.undo", Some(&id));
        assert_eq!(outcome.message.as_deref(), Some("Nothing to undo"));
        run(&services, "edit.redo", Some(&id));
        assert_eq!(services.documents.info(&id).unwrap().title, "Cover");

        run(&services, "file.save", Some(&id));
        assert!(!services.documents.info(&id).unwrap().dirty);

        // a closed or missing document is not an error, just nothing to do
        let outcome = run(&services, "edit.undo", Some("doc-404"));
        assert_eq!(outcome.message.as_deref(), Some("Nothing to undo"));
        let outcome = run(&services, "file.save", None);
        assert_eq!(
            outcome.message.as_deref(),
            Some("Open a document to save it")
        );
        services.shutdown();
    }

    #[test]
    fn view_commands_change_settings() {
        let (_dir, services) = start();
        run(&services, "view.theme.high_contrast", None);
        run(&services, "view.compact.toggle", None);
        run(&services, "view.zoom.in", None);
        run(&services, "view.zoom.in", None);
        let look = services.settings.get().appearance;
        assert_eq!(look.theme, Theme::HighContrast);
        assert!(look.compact);
        assert_eq!(look.ui_scale, 1.25);

        run(&services, "view.zoom.out", None);
        assert_eq!(services.settings.get().appearance.ui_scale, 1.1);
        run(&services, "view.zoom.reset", None);
        assert_eq!(services.settings.get().appearance.ui_scale, 1.0);
        services.shutdown();
    }

    #[test]
    fn zoom_stops_at_the_ends() {
        let (_dir, services) = start();
        for _ in 0..20 {
            run(&services, "view.zoom.out", None);
        }
        assert_eq!(services.settings.get().appearance.ui_scale, ZOOM_STEPS[0]);
        services.shutdown();
    }

    #[test]
    fn unknown_commands_are_errors() {
        let (_dir, services) = start();
        let result =
            services.run_command(&CommandRecord::new("app.launch_rocket", json!({})), None);
        assert!(matches!(result, Err(CommandError::UnknownKind(_))));
        services.shutdown();
    }
}
