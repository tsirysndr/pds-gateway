import {
  Card,
  CardBody,
  CardHeader,
  Chip,
  Progress,
  Snippet,
} from "@heroui/react";
import { Alert, ErrorAlert } from "../components/Alert";
import { useInviteCodes } from "../lib/api";

export function InvitesScreen() {
  const { data, error, isPending } = useInviteCodes();
  const codes = data?.codes ?? [];

  return (
    <Card shadow="none" className="border border-default-200">
      <CardHeader className="flex-col items-start gap-1">
        <h2 className="text-lg font-semibold">Invite codes</h2>
        <p className="text-sm text-foreground-500">
          Codes issued to your account by this server.
        </p>
      </CardHeader>
      <CardBody className="gap-3">
        {isPending && <Progress isIndeterminate aria-label="Loading" size="sm" />}
        {error && <ErrorAlert error={error} />}
        {!isPending && !error && codes.length === 0 && (
          <Alert tone="info">
            This server has issued no invite codes to your account.
          </Alert>
        )}
        {codes.map((code) => {
          const used = code.uses?.length ?? 0;
          const exhausted = used >= code.available;
          return (
            <div
              key={code.code}
              className="flex flex-wrap items-center gap-2 rounded-medium border border-default-200 p-3"
            >
              <Snippet size="sm" hideSymbol className="min-w-0 flex-1">
                {code.code}
              </Snippet>
              <Chip
                size="sm"
                variant="flat"
                color={code.disabled ? "danger" : exhausted ? "default" : "success"}
              >
                {code.disabled
                  ? "disabled"
                  : `${used}/${code.available} used`}
              </Chip>
            </div>
          );
        })}
      </CardBody>
    </Card>
  );
}
