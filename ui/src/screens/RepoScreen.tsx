import { useTranslation } from "react-i18next";
import { useState } from "react";
import {
  Accordion,
  AccordionItem,
  Card,
  CardBody,
  CardHeader,
  Chip,
  Code,
  Progress,
  Select,
  SelectItem,
  Snippet,
} from "@heroui/react";
import { useAtomValue } from "jotai";
import { sessionAtom } from "../atoms/store";
import { Alert, ErrorAlert } from "../components/Alert";
import { useRecords, useRepo } from "../lib/api";

export function RepoScreen() {
  const { t } = useTranslation();
  const session = useAtomValue(sessionAtom);
  const repo = session?.did;
  const [collection, setCollection] = useState<string | undefined>();

  const description = useRepo(repo);
  const collections = description.data?.collections ?? [];
  const selected = collection ?? collections[0];
  const records = useRecords(repo, selected);

  return (
    <div className="flex flex-col gap-4">
      <Card shadow="none" className="border border-default-200">
        <CardHeader className="flex-col items-start gap-1">
          <h2 className="text-lg font-semibold">{t("repo.title")}</h2>
          <Code size="sm">{repo}</Code>
        </CardHeader>
        <CardBody className="gap-3">
          {description.isPending && (
            <Progress isIndeterminate aria-label={t("repo.loading")} size="sm" />
          )}
          {description.error ? <ErrorAlert error={description.error} /> : null}

          {description.data && (
            <div className="flex flex-wrap items-center gap-2 text-sm">
              <Chip size="sm" variant="flat">
                {t("repo.collections", { count: collections.length })}
              </Chip>
              {description.data.handleIsCorrect === false && (
                <Chip size="sm" color="warning" variant="flat">
                  {t("repo.handleBroken")}
                </Chip>
              )}
            </div>
          )}

          {collections.length > 0 && (
            <Select
              label={t("repo.collection")}
              variant="bordered"
              selectedKeys={selected ? [selected] : []}
              disallowEmptySelection
              onSelectionChange={(keys) => {
                const next = Array.from(keys)[0];
                if (typeof next === "string") setCollection(next);
              }}
            >
              {collections.map((name) => (
                <SelectItem key={name}>{name}</SelectItem>
              ))}
            </Select>
          )}
        </CardBody>
      </Card>

      <Card shadow="none" className="border border-default-200">
        <CardBody className="gap-3">
          {records.isPending && selected && (
            <Progress isIndeterminate aria-label={t("repo.loadingRecords")} size="sm" />
          )}
          {records.error ? <ErrorAlert error={records.error} /> : null}
          {!selected && <Alert tone="info">{t("repo.noCollections")}</Alert>}

          {records.data && records.data.records.length === 0 && (
            <Alert tone="info">{t("repo.noRecords", { collection: selected })}</Alert>
          )}

          {records.data && records.data.records.length > 0 && (
            <Accordion variant="splitted" className="px-0">
              {records.data.records.map((record) => (
                <AccordionItem
                  key={record.uri}
                  title={<span className="font-mono text-xs">{rkeyOf(record.uri)}</span>}
                  subtitle={<span className="text-xs">{record.cid.slice(0, 16)}…</span>}
                >
                  <Snippet
                    size="sm"
                    hideSymbol
                    className="w-full overflow-x-auto whitespace-pre font-mono text-xs"
                  >
                    {JSON.stringify(record.value, null, 2)}
                  </Snippet>
                </AccordionItem>
              ))}
            </Accordion>
          )}
        </CardBody>
      </Card>
    </div>
  );
}

function rkeyOf(uri: string) {
  return uri.split("/").pop() ?? uri;
}
