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
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useAtomValue } from "jotai";
import { IconKey } from "@tabler/icons-react";
import { pdsUrlAtom } from "../atoms/store";
import { Alert, ErrorAlert } from "../components/Alert";
import {
  listPasskeys,
  passkeysAvailable,
  UnsupportedError,
  fromBase64Url,
  toBase64Url,
} from "../lib/security";

/// Registration options as the server sends them, base64url-encoded.
type CreationOptions = {
  challenge: string;
  rp: { id?: string; name: string };
  user: { id: string; name: string; displayName: string };
  pubKeyCredParams: { type: "public-key"; alg: number }[];
  timeout?: number;
  excludeCredentials?: { id: string; type: "public-key" }[];
  authenticatorSelection?: AuthenticatorSelectionCriteria;
  attestation?: AttestationConveyancePreference;
};

function toPublicKey(options: CreationOptions): PublicKeyCredentialCreationOptions {
  return {
    ...options,
    challenge: fromBase64Url(options.challenge) as BufferSource,
    user: {
      ...options.user,
      id: fromBase64Url(options.user.id) as BufferSource,
    },
    excludeCredentials: options.excludeCredentials?.map((c) => ({
      ...c,
      id: fromBase64Url(c.id) as BufferSource,
    })),
  };
}

export function PasskeysScreen() {
  const base = useAtomValue(pdsUrlAtom);
  const queries = useQueryClient();
  const supported = passkeysAvailable();

  const list = useQuery({
    queryKey: ["passkeys", base],
    retry: false,
    queryFn: () => listPasskeys(base),
  });

  const register = useMutation({
    mutationFn: async () => {
      const begin = await fetch(new URL("/account/passkeys/register/begin", base), {
        method: "POST",
        credentials: "include",
      });
      if (!begin.ok) throw new UnsupportedError();
      const options = (await begin.json()) as CreationOptions;

      const credential = (await navigator.credentials.create({
        publicKey: toPublicKey(options),
      })) as PublicKeyCredential | null;
      if (!credential) throw new Error("No passkey was created.");

      const attestation = credential.response as AuthenticatorAttestationResponse;
      const finish = await fetch(new URL("/account/passkeys/register/finish", base), {
        method: "POST",
        credentials: "include",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({
          id: credential.id,
          rawId: toBase64Url(credential.rawId),
          type: credential.type,
          response: {
            clientDataJSON: toBase64Url(attestation.clientDataJSON),
            attestationObject: toBase64Url(attestation.attestationObject),
          },
        }),
      });
      if (!finish.ok) throw new Error("The server rejected the passkey.");
    },
    onSuccess: () => queries.invalidateQueries({ queryKey: ["passkeys", base] }),
  });

  if (list.error instanceof UnsupportedError) {
    return (
      <Card shadow="none" className="border border-default-200">
        <CardHeader>
          <h2 className="text-lg font-semibold">Passkeys</h2>
        </CardHeader>
        <CardBody>
          <Alert tone="info">
            This server does not offer passkeys. Like two-factor, passkeys are a
            server feature rather than part of the atproto lexicon.
          </Alert>
        </CardBody>
      </Card>
    );
  }

  return (
    <Card shadow="none" className="border border-default-200">
      <CardHeader className="items-center justify-between">
        <div className="flex flex-col gap-1">
          <h2 className="text-lg font-semibold">Passkeys</h2>
          <p className="text-sm text-foreground-500">
            Sign in with your device instead of a password.
          </p>
        </div>
        <Button
          color="primary"
          startContent={<IconKey size={16} />}
          isDisabled={!supported}
          isLoading={register.isPending}
          onPress={() => register.mutate()}
        >
          Add passkey
        </Button>
      </CardHeader>
      <CardBody className="gap-3">
        {!supported && (
          <Alert tone="info">This browser does not support passkeys.</Alert>
        )}
        {register.error ? <ErrorAlert error={register.error} /> : null}

        <Table aria-label="Passkeys" removeWrapper>
          <TableHeader>
            <TableColumn>NAME</TableColumn>
            <TableColumn>ADDED</TableColumn>
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
                  {item.createdAt ? new Date(item.createdAt).toLocaleString() : "—"}
                </TableCell>
              </TableRow>
            )}
          </TableBody>
        </Table>
      </CardBody>
    </Card>
  );
}
