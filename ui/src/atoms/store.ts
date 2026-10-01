import { atom } from "jotai";
import { atomWithStorage, createJSONStorage } from "jotai/utils";
import type { Session } from "../lib/types";

/// `atomWithStorage` reads eagerly, so a blocked or empty localStorage must not
/// take the app down with it.
function storage<T>() {
  return createJSONStorage<T>(() => {
    try {
      return window.localStorage;
    } catch {
      return undefined as unknown as Storage;
    }
  });
}

/// Base URL of the PDS every request goes to. Defaults to the origin serving
/// this page, which for the gateway means the account's own node is found by
/// routing rather than configuration.
export const pdsUrlAtom = atomWithStorage<string>(
  "pdsgw.pds",
  window.location.origin,
  storage<string>(),
);

export const sessionAtom = atomWithStorage<Session | null>(
  "pdsgw.session",
  null,
  storage<Session | null>(),
);

/// The handle the user last signed in with, used to prefill the form.
export const lastHandleAtom = atomWithStorage<string>(
  "pdsgw.handle",
  "",
  storage<string>(),
);

export const accessTokenAtom = atom((get) => get(sessionAtom)?.accessJwt);

export const isSignedInAtom = atom((get) => get(sessionAtom) !== null);

/// The chosen language, remembered per browser. Empty until chosen, so the
/// browser's own preference wins on a first visit.
export const languageAtom = atomWithStorage<string>("pdsgw.language", "");
