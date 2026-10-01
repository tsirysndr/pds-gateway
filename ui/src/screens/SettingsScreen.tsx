import { Card, CardBody, CardHeader, Code, Snippet } from "@heroui/react";
import { useQuery } from "@tanstack/react-query";
import { useAtomValue } from "jotai";
import { pdsUrlAtom } from "../atoms/store";
import { PdsSelect } from "../components/PdsSelect";
import { Alert, ErrorAlert } from "../components/Alert";
import { describeServer } from "../lib/servers";

export function SettingsScreen() {
  const pds = useAtomValue(pdsUrlAtom);
  const { data, error } = useQuery({
    queryKey: ["describeServer", pds],
    queryFn: () => describeServer(pds),
  });

  return (
    <div className="flex flex-col gap-4">
      <Card shadow="none" className="border border-default-200">
        <CardHeader className="flex-col items-start gap-1">
          <h2 className="text-lg font-semibold">Server</h2>
          <p className="text-sm text-foreground-500">
            Every request goes to this server.
          </p>
        </CardHeader>
        <CardBody className="gap-3">
          <PdsSelect />
          <Snippet size="sm" hideSymbol className="w-full overflow-x-auto">
            {pds}
          </Snippet>
          <Alert tone="info">
            Choosing a server here overrides the one detected from your handle.
            Sign in again to let detection pick it for you.
          </Alert>
        </CardBody>
      </Card>

      <Card shadow="none" className="border border-default-200">
        <CardHeader>
          <h3 className="font-medium">What it reports</h3>
        </CardHeader>
        <CardBody className="gap-2 text-sm">
          {error && <ErrorAlert error={error} />}
          <Row label="DID">{data?.did ? <Code size="sm">{data.did}</Code> : "—"}</Row>
          <Row label="Handle domains">
            {data?.availableUserDomains?.join(", ") ?? "—"}
          </Row>
          <Row label="Invite required">
            {data?.inviteCodeRequired ? "yes" : "no"}
          </Row>
          <Row label="Blob limit">
            {data?.blobUploadLimit
              ? `${Math.round(data.blobUploadLimit / 1024 / 1024)} MB`
              : "—"}
          </Row>
          <Row label="Contact">{data?.contact?.email ?? "—"}</Row>
        </CardBody>
      </Card>
    </div>
  );
}

function Row({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="flex flex-wrap gap-2">
      <span className="w-36 shrink-0 text-foreground-500">{label}</span>
      <span className="min-w-0 break-words">{children}</span>
    </div>
  );
}
