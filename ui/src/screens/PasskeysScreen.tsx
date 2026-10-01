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
const schema = z.object({
  name: z.string().trim().max(64, "Keep the name under 64 characters").optional(),
  password: z.string().min(1, "Enter your password"),
  code: z.string().trim().optional(),
});

export function PasskeysScreen() {
  const client = useClient();
  const queries = useQueryClient();
  const supported = passkeysAvailable();
  const [added, setAdded] = useState<string | null>(null);

  const form = useForm<z.infer<typeof schema>>({
    resolver: zodResolver(schema),
    defaultValues: { name: "", password: "", code: "" },
  });

  const list = useQuery({
    queryKey: ["passkeys", client.base],
    retry: false,
    queryFn: () => listPasskeys(client),
  });

  const register = useMutation({
    mutationFn: async (values: z.infer<typeof schema>) => {
      const started = await beginPasskeyRegistration(client, {
        password: values.password,
        code: values.code || undefined,
        name: values.name || undefined,
      });

      const credential = (await navigator.credentials.create({
        publicKey: toPublicKey(started.publicKey),
      })) as PublicKeyCredential | null;
      if (!credential) throw new Error("No passkey was created.");

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
          <h2 className="text-lg font-semibold">Passkeys</h2>
        </CardHeader>
        <CardBody>
          <Alert tone="info">
            This server does not implement <code>social.rocksky.auth</code>.
            Like two-factor, passkeys are a server feature rather than part of
            the atproto lexicon.
          </Alert>
        </CardBody>
      </Card>
    );
  }

  return (
    <div className="flex flex-col gap-4">
      <Card shadow="none" className="border border-default-200">
        <CardHeader className="flex-col items-start gap-1">
          <h2 className="text-lg font-semibold">Passkeys</h2>
          <p className="text-sm text-foreground-500">
            Sign in with your device instead of a password.
          </p>
        </CardHeader>
        <CardBody className="gap-3">
          {!supported && (
            <Alert tone="info">This browser does not support passkeys.</Alert>
          )}

          <form
            className="flex flex-col gap-3"
            onSubmit={form.handleSubmit((values) => register.mutate(values))}
          >
            <div className="flex flex-col gap-3 sm:flex-row">
              <Field
                className="sm:flex-1"
                label="Name"
                placeholder="MacBook"
                error={form.formState.errors.name}
                {...form.register("name")}
              />
              <Field
                className="sm:flex-1"
                label="Password"
                type="password"
                autoComplete="current-password"
                error={form.formState.errors.password}
                {...form.register("password")}
              />
              <Field
                className="sm:w-36"
                label="Code"
                description="If two-factor is on"
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
              Add passkey
            </Button>
          </form>

          {added && <Alert tone="success">Added {added}.</Alert>}
          {register.error ? <ErrorAlert error={register.error} /> : null}
        </CardBody>
      </Card>

      <Card shadow="none" className="border border-default-200">
        <CardBody className="gap-3">
          {list.error ? <ErrorAlert error={list.error} /> : null}
          <Table aria-label="Passkeys" removeWrapper>
            <TableHeader>
              <TableColumn>NAME</TableColumn>
              <TableColumn>ADDED</TableColumn>
              <TableColumn> </TableColumn>
            </TableHeader>
            <TableBody
              isLoading={list.isPending}
              emptyContent="No passkeys yet."
              items={list.data ?? []}
            >
              {(item) => (
                <TableRow key={item.id}>
                  <TableCell>{item.name ?? item.id.slice(0, 12)}</TableCell>
                  <TableCell className="text-foreground-500">
                    {item.createdAt
                      ? new Date(item.createdAt).toLocaleString()
                      : "—"}
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
                            message: "Enter your password to remove a passkey",
                          });
                          return;
                        }
                        remove.mutate({ id: item.id, password });
                      }}
                    >
                      Remove
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
