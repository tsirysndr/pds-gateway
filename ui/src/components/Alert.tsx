import { IconAlertTriangle, IconCheck, IconInfoCircle } from "@tabler/icons-react";

type Tone = "danger" | "success" | "info";

const tones: Record<Tone, { box: string; Icon: typeof IconCheck }> = {
  danger: { box: "bg-danger-50 text-danger-700 dark:text-danger-400", Icon: IconAlertTriangle },
  success: { box: "bg-success-50 text-success-700 dark:text-success-400", Icon: IconCheck },
  info: { box: "bg-default-100 text-foreground-600", Icon: IconInfoCircle },
};

export function Alert({
  tone = "info",
  title,
  children,
}: {
  tone?: Tone;
  title?: string;
  children?: React.ReactNode;
}) {
  const { box, Icon } = tones[tone];
  return (
    <div className={`flex gap-2 rounded-medium p-3 text-sm ${box}`} role="status">
      <Icon size={18} className="mt-0.5 shrink-0" />
      <div className="min-w-0">
        {title && <p className="font-medium">{title}</p>}
        {children && <div className="break-words">{children}</div>}
      </div>
    </div>
  );
}

/// Renders whatever an XRPC call threw, without assuming a shape.
export function ErrorAlert({ error }: { error: unknown }) {
  if (!error) return null;
  const message = error instanceof Error ? error.message : String(error);
  const name =
    typeof error === "object" && error !== null && "error" in error
      ? String((error as { error: unknown }).error)
      : undefined;
  return (
    <Alert tone="danger" title={name}>
      {message}
    </Alert>
  );
}
