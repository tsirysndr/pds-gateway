import { Button, Card, CardBody, CardHeader, Chip, Code, Divider, Snippet } from "@heroui/react";
import { useForm } from "react-hook-form";
import { zodResolver } from "@hookform/resolvers/zod";
import { z } from "zod";
import { useAtomValue } from "jotai";
import { sessionAtom } from "../atoms/store";
import { Alert, ErrorAlert } from "../components/Alert";
import { Field } from "../components/Field";
import {
  useRequestEmailConfirmation,
  useSession,
  useUpdateHandle,
} from "../lib/api";

const handleSchema = z.object({
  handle: z
    .string()
    .trim()
    .min(3, "Enter the new handle")
    .refine((v) => v.includes("."), "Handles look like alice.example.com"),
});

export function AccountScreen() {
  const stored = useAtomValue(sessionAtom);
  const { data, error, isPending } = useSession();
  const account = data ?? stored;

  const rename = useUpdateHandle();
  const confirmEmail = useRequestEmailConfirmation();

  const form = useForm<z.infer<typeof handleSchema>>({
    resolver: zodResolver(handleSchema),
    defaultValues: { handle: account?.handle ?? "" },
  });

  return (
    <div className="flex flex-col gap-4">
      <Card shadow="none" className="border border-default-200">
        <CardHeader className="flex-col items-start gap-1">
          <h2 className="text-lg font-semibold">{account?.handle ?? "Account"}</h2>
          <Code size="sm">{account?.did}</Code>
        </CardHeader>
        <CardBody className="gap-3">
          {error && <ErrorAlert error={error} />}
          {isPending && !stored && <p className="text-sm text-foreground-500">Loading…</p>}

          <div className="flex flex-wrap items-center gap-2 text-sm">
            <span className="text-foreground-500">Email</span>
            <span>{account?.email ?? "—"}</span>
            {account?.email && (
              <Chip
                size="sm"
                variant="flat"
                color={account.emailConfirmed ? "success" : "warning"}
              >
                {account.emailConfirmed ? "confirmed" : "unconfirmed"}
              </Chip>
            )}
            {account?.email && !account.emailConfirmed && (
              <Button
                size="sm"
                variant="flat"
                isLoading={confirmEmail.isPending}
                onPress={() => confirmEmail.mutate()}
              >
                Send confirmation
              </Button>
            )}
          </div>
          {confirmEmail.isSuccess && (
            <Alert tone="success">Confirmation email sent.</Alert>
          )}
          {confirmEmail.error && <ErrorAlert error={confirmEmail.error} />}
        </CardBody>
      </Card>

      <Card shadow="none" className="border border-default-200">
        <CardHeader>
          <h3 className="font-medium">Change handle</h3>
        </CardHeader>
        <CardBody>
          <form
            className="flex flex-col gap-3 sm:flex-row sm:items-start"
            onSubmit={form.handleSubmit((values) => rename.mutate(values.handle))}
          >
            <Field
              className="sm:flex-1"
              label="New handle"
              autoCapitalize="none"
              spellCheck="false"
              error={form.formState.errors.handle}
              {...form.register("handle")}
            />
            <Button
              type="submit"
              color="primary"
              className="sm:mt-3"
              isLoading={rename.isPending}
            >
              Update
            </Button>
          </form>
          {rename.isSuccess && <Alert tone="success">Handle updated.</Alert>}
          {rename.error && <ErrorAlert error={rename.error} />}
        </CardBody>
      </Card>

      <Card shadow="none" className="border border-default-200">
        <CardHeader>
          <h3 className="font-medium">Export</h3>
        </CardHeader>
        <CardBody className="gap-2">
          <p className="text-sm text-foreground-500">
            Your repository as a CAR file, straight from the PDS that holds it.
          </p>
          <Snippet size="sm" hideSymbol className="w-full overflow-x-auto">
            {`com.atproto.sync.getRepo?did=${account?.did ?? ""}`}
          </Snippet>
          <Divider />
          <p className="text-xs text-foreground-500">
            Deactivating or deleting an account is done from the server that hosts
            it.
          </p>
        </CardBody>
      </Card>
    </div>
  );
}
