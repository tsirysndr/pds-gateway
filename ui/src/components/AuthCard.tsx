import { Card, CardBody, CardHeader } from "@heroui/react";

export function AuthCard({
  title,
  subtitle,
  children,
  footer,
}: {
  title: string;
  subtitle?: string;
  children: React.ReactNode;
  footer?: React.ReactNode;
}) {
  return (
    <div className="mx-auto flex min-h-svh w-full max-w-md flex-col justify-center gap-4 p-4">
      <Card className="border border-default-200" shadow="none">
        <CardHeader className="flex-col items-start gap-1 pb-0">
          <h1 className="text-xl font-semibold">{title}</h1>
          {subtitle && <p className="text-sm text-foreground-500">{subtitle}</p>}
        </CardHeader>
        <CardBody className="gap-4">{children}</CardBody>
      </Card>
      {footer && <div className="text-center text-sm text-foreground-500">{footer}</div>}
    </div>
  );
}
