export type Session = {
  did: string;
  handle: string;
  accessJwt: string;
  refreshJwt: string;
  email?: string;
  emailConfirmed?: boolean;
  active?: boolean;
};

export type ServerDescription = {
  did?: string;
  availableUserDomains?: string[];
  inviteCodeRequired?: boolean;
  blobUploadLimit?: number;
  contact?: { email?: string };
  links?: { privacyPolicy?: string; termsOfService?: string };
};

export type RepoDescription = {
  handle: string;
  did: string;
  didDoc?: unknown;
  collections?: string[];
  handleIsCorrect?: boolean;
};

export type RecordEntry = { uri: string; cid: string; value: Record<string, unknown> };
export type AppPassword = { name: string; createdAt: string; privileged?: boolean };
export type InviteCode = {
  code: string;
  available: number;
  disabled?: boolean;
  uses?: { usedBy: string; usedAt: string }[];
};
