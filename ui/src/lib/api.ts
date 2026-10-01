import { useAtomValue, useSetAtom } from "jotai";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { pdsUrlAtom, sessionAtom } from "../atoms/store";
import { createClient, XrpcError } from "./xrpc";
import type {
  AppPassword,
  InviteCode,
  RecordEntry,
  RepoDescription,
  Session,
} from "./types";

/// A client bound to the selected PDS and the current session.
export function useClient() {
  const base = useAtomValue(pdsUrlAtom);
  const session = useAtomValue(sessionAtom);
  return createClient(base, session?.accessJwt);
}

/// Signs the session out of local state when the PDS says the token is done.
export function useAuthGuard() {
  const setSession = useSetAtom(sessionAtom);
  return (error: unknown) => {
    if (error instanceof XrpcError && error.isAuthExpired) setSession(null);
  };
}

export function useSession() {
  const client = useClient();
  const session = useAtomValue(sessionAtom);
  const guard = useAuthGuard();

  return useQuery({
    queryKey: ["getSession", client.base, session?.did],
    enabled: session !== null,
    retry: false,
    queryFn: async () => {
      try {
        return await client.get<Session>("com.atproto.server.getSession");
      } catch (error) {
        guard(error);
        throw error;
      }
    },
  });
}

export function useRepo(repo: string | undefined) {
  const client = useClient();
  return useQuery({
    queryKey: ["describeRepo", client.base, repo],
    enabled: Boolean(repo),
    queryFn: () =>
      client.get<RepoDescription>("com.atproto.repo.describeRepo", { repo }),
  });
}

export function useRecords(repo: string | undefined, collection: string | undefined) {
  const client = useClient();
  return useQuery({
    queryKey: ["listRecords", client.base, repo, collection],
    enabled: Boolean(repo && collection),
    queryFn: () =>
      client.get<{ records: RecordEntry[]; cursor?: string }>(
        "com.atproto.repo.listRecords",
        { repo, collection, limit: 50 },
      ),
  });
}

export function useAppPasswords() {
  const client = useClient();
  const session = useAtomValue(sessionAtom);
  return useQuery({
    queryKey: ["listAppPasswords", client.base, session?.did],
    enabled: session !== null,
    queryFn: () =>
      client.get<{ passwords: AppPassword[] }>(
        "com.atproto.server.listAppPasswords",
      ),
  });
}

export function useCreateAppPassword() {
  const client = useClient();
  const queries = useQueryClient();
  return useMutation({
    mutationFn: (input: { name: string; privileged?: boolean }) =>
      client.post<AppPassword & { password: string }>(
        "com.atproto.server.createAppPassword",
        input,
      ),
    onSuccess: () => queries.invalidateQueries({ queryKey: ["listAppPasswords"] }),
  });
}

export function useRevokeAppPassword() {
  const client = useClient();
  const queries = useQueryClient();
  return useMutation({
    mutationFn: (name: string) =>
      client.post("com.atproto.server.revokeAppPassword", { name }),
    onSuccess: () => queries.invalidateQueries({ queryKey: ["listAppPasswords"] }),
  });
}

export function useInviteCodes() {
  const client = useClient();
  const session = useAtomValue(sessionAtom);
  return useQuery({
    queryKey: ["inviteCodes", client.base, session?.did],
    enabled: session !== null,
    retry: false,
    queryFn: () =>
      client.get<{ codes: InviteCode[] }>(
        "com.atproto.server.getAccountInviteCodes",
      ),
  });
}

export function useUpdateHandle() {
  const client = useClient();
  const setSession = useSetAtom(sessionAtom);
  const queries = useQueryClient();
  return useMutation({
    mutationFn: (handle: string) =>
      client.post("com.atproto.identity.updateHandle", { handle }),
    onSuccess: (_data, handle) => {
      setSession((current) => (current ? { ...current, handle } : current));
      queries.invalidateQueries();
    },
  });
}

export function useRequestEmailConfirmation() {
  const client = useClient();
  return useMutation({
    mutationFn: () => client.post("com.atproto.server.requestEmailConfirmation"),
  });
}

export function useRefreshSession() {
  const base = useAtomValue(pdsUrlAtom);
  const session = useAtomValue(sessionAtom);
  const setSession = useSetAtom(sessionAtom);

  return useMutation({
    mutationFn: async () => {
      if (!session) throw new Error("not signed in");
      // Refresh is authorised by the refresh token, not the access token.
      const client = createClient(base, session.refreshJwt);
      return client.post<Session>("com.atproto.server.refreshSession");
    },
    onSuccess: (next) => setSession((current) => ({ ...current, ...next })),
  });
}

export function useSignOut() {
  const base = useAtomValue(pdsUrlAtom);
  const session = useAtomValue(sessionAtom);
  const setSession = useSetAtom(sessionAtom);
  const queries = useQueryClient();

  return useMutation({
    mutationFn: async () => {
      if (session) {
        const client = createClient(base, session.refreshJwt);
        // A server-side failure must not trap the user in a signed-in UI.
        await client.post("com.atproto.server.deleteSession").catch(() => {});
      }
    },
    onSettled: () => {
      setSession(null);
      queries.clear();
    },
  });
}
