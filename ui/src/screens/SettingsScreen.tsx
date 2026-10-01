import { useTranslation } from "react-i18next";
import { Card, CardBody, CardHeader, Code, Snippet } from "@heroui/react";
import { useQuery } from "@tanstack/react-query";
import { useAtomValue } from "jotai";
import { pdsUrlAtom } from "../atoms/store";
import { PdsSelect } from "../components/PdsSelect";
import { Alert, ErrorAlert } from "../components/Alert";
import { describeServer } from "../lib/servers";

export function SettingsScreen() {
  const { t } = useTranslation();
  const pds = useAtomValue(pdsUrlAtom);
  const { data, error } = useQuery({
    queryKey: ["describeServer", pds],
    queryFn: () => describeServer(pds),
  });

  return (
    <div className="flex flex-col gap-4">
      <Card shadow="none" className="border border-default-200">
        <CardHeader className="flex-col items-start gap-1">
          <h2 className="text-lg font-semibold">{t("settings.serverTitle")}</h2>
          <p className="text-sm text-foreground-500">{t("settings.serverSubtitle")}</p>
        </CardHeader>
        <CardBody className="gap-3">
          <PdsSelect />
          <Snippet size="sm" hideSymbol className="w-full overflow-x-auto">
            {pds}
          </Snippet>
          <Alert tone="info">
            {t("settings.overrideNote")}
          </Alert>
        </CardBody>
      </Card>

      <Card shadow="none" className="border border-default-200">
        <CardHeader>
          <h3 className="font-medium">{t("settings.reportsTitle")}</h3>
        </CardHeader>
        <CardBody className="gap-2 text-sm">
          {error && <ErrorAlert error={error} />}
          <Row label={t("settings.did")}>{data?.did ? <Code size="sm">{data.did}</Code> : "—"}</Row>
          <Row label={t("settings.handleDomains")}>
            {data?.availableUserDomains?.join(", ") ?? "—"}
          </Row>
          <Row label={t("settings.inviteRequired")}>
            {data?.inviteCodeRequired ? t("settings.yes") : t("settings.no")}
          </Row>
          <Row label={t("settings.blobLimit")}>
            {data?.blobUploadLimit
              ? `${Math.round(data.blobUploadLimit / 1024 / 1024)} MB`
              : "—"}
          </Row>
          <Row label={t("settings.contact")}>{data?.contact?.email ?? "—"}</Row>
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
