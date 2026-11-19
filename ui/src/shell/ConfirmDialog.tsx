import { Button } from "../components/Button";
import { Dialog } from "../components/Dialog";
import { useUi } from "../state/ui";

/** the one question dialog, driven by askConfirm() */
export function ConfirmDialog() {
  const request = useUi((s) => s.confirm);
  if (!request) return null;
  return (
    <Dialog
      open
      onOpenChange={(open) => {
        if (!open) request.resolve("cancel");
      }}
      title={request.title}
      description={request.body}
      width="26rem"
      footer={
        <>
          <Button
            variant="ghost"
            tooltip="Go back without changing anything"
            onClick={() => {
              request.resolve("cancel");
            }}
          >
            Cancel
          </Button>
          {request.altLabel && (
            <Button
              tooltip={request.altLabel}
              onClick={() => {
                request.resolve("alt");
              }}
            >
              {request.altLabel}
            </Button>
          )}
          <Button
            variant={request.danger ? "danger" : "primary"}
            tooltip={request.confirmLabel}
            onClick={() => {
              request.resolve("confirm");
            }}
          >
            {request.confirmLabel}
          </Button>
        </>
      }
    />
  );
}
