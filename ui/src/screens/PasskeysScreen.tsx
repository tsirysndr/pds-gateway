import { formatWhen } from "../lib/when";
import { useState } from "react";
import {
  Button,
  Card,
  CardBody,
  CardHeader,
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
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { IconKey } from "@tabler/icons-react";
import { Alert, ErrorAlert } from "../components/Alert";
import { Field } from "../components/Field";
import { useClient } from "../lib/api";
import {
  beginPasskeyRegistration,
  credentialJson,
  deletePasskey,
  finishPasskeyRegistration,
  isUnsupported,
  listPasskeys,
  passkeysAvailable,
  toPublicKey,
} from "../lib/security";

/// Adding a passkey adds a way to sign in, so it takes the password — an access
/// token proves the session, not the owner.
const makeSchema = (t: (key: string) => string) =>
  z.object({
  name: z.string().trim().max(64, t("passkeys.errorName")).optional(),
  password: z.string().min(1, t("security.errorPasswordRequired")),
  code: z.string().trim().optional(),
});

export function PasskeysScreen() {
  const { t } = useTranslation();
  const client = useClient();
  const queries = useQueryClient();
  const supported = passkeysAvailable();
  const [added, setAdded] = useState<string | null>(null);

  const form = useForm<z.infer<ReturnType<typeof makeSchema>>>({
    resolver: zodResolver(makeSchema(t)),
    defaultValues: { name: "", password: "", code: "" },
  });

  const list = useQuery({
    queryKey: ["passkeys", client.base],
    retry: false,
    queryFn: () => listPasskeys(client),
  });

  const register = useMutation({
    mutationFn: async (values: z.infer<ReturnType<typeof makeSchema>>) => {
      const started = await beginPasskeyRegistration(client, {
        password: values.password,
        code: values.code || undefined,
        name: values.name || undefined,
      });

      const credential = (await navigator.credentials.create({
        publicKey: toPublicKey(started.publicKey),
      })) as PublicKeyCredential | null;
      if (!credential) throw new Error(t("passkeys.noneCreated"));

      return finishPasskeyRegistration(client, {
        requestId: started.requestId,
        credential: credentialJson(credential),
      });
    },
    onSuccess: (result) => {
      setAdded(result.passkey.name ?? result.passkey.id);
      form.reset();
      queries.invalidateQueries({ queryKey: ["passkeys", client.base] });
    },
  });

  const remove = useMutation({
    mutationFn: ({ id, password }: { id: string; password: string }) =>
      deletePasskey(client, id, password),
    onSuccess: () =>
      queries.invalidateQueries({ queryKey: ["passkeys", client.base] }),
  });

  if (isUnsupported(list.error)) {
    return (
      <Card shadow="none" className="border border-default-200">
        <CardHeader>
          <h2 className="text-lg font-semibold">{t("passkeys.title")}</h2>
        </CardHeader>
        <CardBody>
          <Alert tone="info">
            {t("security.notImplementedBefore")} <code>social.rocksky.auth</code>.{" "}
            {t("passkeys.notImplementedAfter")}
          </Alert>
        </CardBody>
      </Card>
    );
  }

  return (
    <div className="flex flex-col gap-4">
      <Card shadow="none" className="border border-default-200">
        <CardHeader className="flex-col items-start gap-1">
          <h2 className="text-lg font-semibold">{t("passkeys.title")}</h2>
          <p className="text-sm text-foreground-500">{t("passkeys.subtitle")}</p>
        </CardHeader>
        <CardBody className="gap-3">
          {!supported && (
            <Alert tone="info">{t("passkeys.unsupported")}</Alert>
          )}

          <form
            className="flex flex-col gap-3"
            onSubmit={form.handleSubmit((values) => register.mutate(values))}
          >
            <div className="flex flex-col gap-3 sm:flex-row">
              <Field
                className="sm:flex-1"
                label={t("common.name")}
                placeholder={t("passkeys.namePlaceholder")}
                error={form.formState.errors.name}
                {...form.register("name")}
              />
              <Field
                className="sm:flex-1"
                label={t("common.password")}
                type="password"
                autoComplete="current-password"
                error={form.formState.errors.password}
                {...form.register("password")}
              />
              <Field
                className="sm:w-36"
                label={t("login.factorLabel")}
                description={t("passkeys.codeDescription")}
                inputMode="numeric"
                autoComplete="one-time-code"
                error={form.formState.errors.code}
                {...form.register("code")}
              />
            </div>
            <Button
              type="submit"
              color="primary"
              className="self-start"
              startContent={<IconKey size={16} />}
              isDisabled={!supported}
              isLoading={register.isPending}
            >
              {t("passkeys.add")}
            </Button>
          </form>

          {added && <Alert tone="success">{t("passkeys.added", { name: added })}</Alert>}
          {register.error ? <ErrorAlert error={register.error} /> : null}
        </CardBody>
      </Card>

      <Card shadow="none" className="border border-default-200">
        <CardBody className="gap-3">
          {list.error ? <ErrorAlert error={list.error} /> : null}
          <Table aria-label={t("passkeys.title")} removeWrapper>
            <TableHeader>
              <TableColumn>{t("passkeys.columnName")}</TableColumn>
              <TableColumn>{t("passkeys.columnAdded")}</TableColumn>
              <TableColumn> </TableColumn>
            </TableHeader>
            <TableBody
              isLoading={list.isPending}
              emptyContent={t("passkeys.empty")}
              items={list.data ?? []}
            >
              {(item) => (
                <TableRow key={item.id}>
                  <TableCell>{item.name ?? item.id.slice(0, 12)}</TableCell>
                  <TableCell className="text-foreground-500">
                    {formatWhen(item.createdAt) ?? "—"}
                  </TableCell>
                  <TableCell className="text-right">
                    <Button
                      size="sm"
                      variant="flat"
                      color="danger"
                      isLoading={remove.isPending && remove.variables?.id === item.id}
                      onPress={() => {
                        // Removing a way to sign in takes the password too.
                        const password = form.getValues("password");
                        if (!password) {
                          form.setError("password", {
                            message: t("passkeys.errorRemovePassword"),
                          });
                          return;
                        }
                        remove.mutate({ id: item.id, password });
                      }}
                    >
                      {t("common.remove")}
                    </Button>
                  </TableCell>
                </TableRow>
              )}
            </TableBody>
          </Table>
          {remove.error ? <ErrorAlert error={remove.error} /> : null}
        </CardBody>
      </Card>
    </div>
  );
}
