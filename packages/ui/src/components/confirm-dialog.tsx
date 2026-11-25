"use client";

import { AlertTriangle, Trash2 } from "lucide-react";
import { useState, type ReactNode } from "react";
import { Button } from "./button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "./dialog";
import { cn } from "../lib/utils";

export interface ConfirmDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  title: string;
  description: ReactNode;
  /** Optional target identifier highlighted in a mono pill (e.g. app/db/server name). */
  target?: string;
  confirmLabel?: string;
  cancelLabel?: string;
  tone?: "danger" | "warning";
  onConfirm: () => void | Promise<void>;
}

export function ConfirmDialog({
  open,
  onOpenChange,
  title,
  description,
  target,
  confirmLabel = "Confirm",
  cancelLabel = "Cancel",
  tone = "danger",
  onConfirm,
}: ConfirmDialogProps) {
  const [busy, setBusy] = useState(false);

  async function handleConfirm() {
    setBusy(true);
    try {
      await onConfirm();
      onOpenChange(false);
    } finally {
      setBusy(false);
    }
  }

  const isDanger = tone === "danger";

  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        if (!busy) onOpenChange(next);
      }}
    >
      <DialogContent size="sm" className="p-6">
        <div className="flex items-start gap-3.5 pr-6">
          <div
            className={cn(
              "mt-0.5 flex size-9 shrink-0 items-center justify-center rounded-lg",
              isDanger
                ? "bg-destructive-light text-fail"
                : "bg-warn/15 text-warn",
            )}
          >
            {isDanger ? (
              <Trash2 className="size-4.5" strokeWidth={1.75} />
            ) : (
              <AlertTriangle className="size-4.5" strokeWidth={1.75} />
            )}
          </div>
          <DialogHeader className="mb-0 min-w-0 flex-1">
            <DialogTitle>{title}</DialogTitle>
            <DialogDescription className="mt-1 leading-relaxed">
              {description}
            </DialogDescription>
          </DialogHeader>
        </div>

        {target ? (
          <div className="mt-4 rounded-lg border border-border/80 bg-surface px-3.5 py-2 font-mono text-[13px] font-medium text-foreground">
            {target}
          </div>
        ) : null}

        <DialogFooter className="mt-6">
          <Button
            type="button"
            variant="ghost"
            disabled={busy}
            onClick={() => onOpenChange(false)}
          >
            {cancelLabel}
          </Button>
          <Button
            type="button"
            variant={isDanger ? "destructive" : "primary"}
            loading={busy}
            onClick={() => void handleConfirm()}
          >
            {confirmLabel}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
