import { formatWhen } from "../lib/when";
import { useState } from "react";
import {
  Button,
  Card,
  CardBody,
  CardHeader,
  Checkbox,
  Snippet,
  Table,
  TableBody,
  TableCell,
  TableColumn,
  TableHeader,
  TableRow,
} from "@heroui/react";
import { useForm } from "react-hook-form";
import { useTranslation } from "react-i18next";
import { zodResolver } from "@hookform/resolvers/zod";
import { z } from "zod";
import { Alert, ErrorAlert } from "../components/Alert";
import { Field } from "../components/Field";
import {
  useAppPasswords,
  useCreateAppPassword,
  useRevokeAppPassword,
} from "../lib/api";

const makeSchema = (t: (key: string) => string) =>
  z.object({
  name: z.string().trim().min(1, t("appPasswords.errorName")),
  privileged: z.boolean(),
});

export function AppPasswordsScreen() {
  const { t } = useTranslation();
  const list = useAppPasswords();
  const create = useCreateAppPassword();
  const revoke = useRevokeAppPassword();
  const [issued, setIssued] = useState<string | null>(null);

  const form = useForm<z.infer<ReturnType<typeof makeSchema>>>({
    resolver: zodResolver(makeSchema(t)),
    defaultValues: { name: "", privileged: false },
  });

  return (
    <div className="flex flex-col gap-4">
      <Card shadow="none" className="border border-default-200">
        <CardHeader className="flex-col items-start gap-1">
          <h2 className="text-lg font-semibold">{t("appPasswords.title")}</h2>
          <p className="text-sm text-foreground-500">{t("appPasswords.subtitle")}</p>
        </CardHeader>
        <CardBody className="gap-3">
          <form
            className="flex flex-col gap-3"
            onSubmit={form.handleSubmit((values) =>
              create.mutate(values, {
                onSuccess: (result) => {
                  setIssued(result.password);
                  form.reset();
                },
              }),
            )}
          >
            <div className="flex flex-col gap-3 sm:flex-row sm:items-start">
              <Field
                className="sm:flex-1"
                label={t("common.name")}
                placeholder={t("appPasswords.namePlaceholder")}
                error={form.formState.errors.name}
                {...form.register("name")}
              />
              <Button
                type="submit"
                color="primary"
                className="sm:mt-3"
                isLoading={create.isPending}
              >
                {t("appPasswords.create")}
              </Button>
            </div>
            <Checkbox size="sm" {...form.register("privileged")}>
              {t("appPasswords.allowDms")}
            </Checkbox>
          </form>

          {issued && (
            <Alert tone="success" title={t("appPasswords.copyOnce")}>
              <Snippet size="sm" hideSymbol className="mt-1 w-full overflow-x-auto">
                {issued}
              </Snippet>
            </Alert>
          )}
          {create.error && <ErrorAlert error={create.error} />}
        </CardBody>
      </Card>

      <Card shadow="none" className="border border-default-200">
        <CardBody>
          {list.error && <ErrorAlert error={list.error} />}
          <Table aria-label={t("appPasswords.title")} removeWrapper>
            <TableHeader>
              <TableColumn>{t("appPasswords.columnName")}</TableColumn>
              <TableColumn>{t("appPasswords.columnCreated")}</TableColumn>
              <TableColumn> </TableColumn>
            </TableHeader>
            <TableBody
              isLoading={list.isPending}
              emptyContent={t("appPasswords.empty")}
              items={list.data?.passwords ?? []}
            >
              {(item) => (
                <TableRow key={item.name}>
                  <TableCell>{item.name}</TableCell>
                  <TableCell className="text-foreground-500">
                    {formatWhen(item.createdAt) ?? "—"}
                  </TableCell>
                  <TableCell className="text-right">
                    <Button
                      size="sm"
                      variant="flat"
                      color="danger"
                      isLoading={revoke.isPending && revoke.variables === item.name}
                      onPress={() => revoke.mutate(item.name)}
                    >
                      Revoke
                    </Button>
                  </TableCell>
                </TableRow>
              )}
            </TableBody>
          </Table>
          {revoke.error && <ErrorAlert error={revoke.error} />}
        </CardBody>
      </Card>
    </div>
  );
}
