import { Card, CardBody, CardHeader } from "@heroui/react";
import { AuthBackdrop } from "./AuthBackdrop";

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
    <>
      <AuthBackdrop />
      {/* Above the backdrop, which is fixed at z-0. */}
      <div className="relative z-10 mx-auto flex min-h-svh w-full max-w-md flex-col justify-center gap-4 p-4">
        {/* Opaque, so the drawing behind it never reduces the form's contrast. */}
        <Card className="border border-default-200 bg-content1" shadow="sm">
          <CardHeader className="flex-col items-start gap-1 pb-0">
            <h1 className="text-xl font-semibold">{title}</h1>
            {subtitle && <p className="text-sm text-foreground-500">{subtitle}</p>}
          </CardHeader>
          <CardBody className="gap-4">{children}</CardBody>
        </Card>
        {footer && (
          <div className="rounded-medium bg-content1/80 px-3 py-2 text-center text-sm text-foreground-500 backdrop-blur">
            {footer}
          </div>
        )}
      </div>
    </>
  );
}
