import {
  Button,
  Chip,
  Navbar,
  NavbarBrand,
  NavbarContent,
  NavbarItem,
  Tab,
  Tabs,
} from "@heroui/react";
import { IconLogout } from "@tabler/icons-react";
import { useAtomValue } from "jotai";
import { useTranslation } from "react-i18next";
import { LanguageSelect } from "./LanguageSelect";
import { pdsUrlAtom, sessionAtom } from "../atoms/store";
import { hostOf } from "../lib/servers";
import { useSignOut } from "../lib/api";

const TABS = ["account", "repo", "sessions", "security", "passkeys", "invites", "settings"];

export function Layout({
  route,
  onNavigate,
  children,
}: {
  route: string;
  onNavigate: (route: string) => void;
  children: React.ReactNode;
}) {
  const session = useAtomValue(sessionAtom);
  const pds = useAtomValue(pdsUrlAtom);
  const signOut = useSignOut();
  const { t } = useTranslation();

  return (
    <div className="min-h-svh">
      <Navbar maxWidth="xl" isBordered>
        <NavbarBrand className="gap-2">
          <span className="font-semibold">{t("common.appName")}</span>
          <Chip size="sm" variant="flat">{hostOf(pds)}</Chip>
        </NavbarBrand>
        <NavbarContent justify="end">
          <NavbarItem className="hidden text-sm text-foreground-500 sm:flex">
            {session?.handle}
          </NavbarItem>
          <NavbarItem className="hidden sm:flex">
            <LanguageSelect />
          </NavbarItem>
          <NavbarItem>
            <Button
              size="sm"
              variant="flat"
              startContent={<IconLogout size={16} />}
              isLoading={signOut.isPending}
              onPress={() => signOut.mutate()}
            >
              {t("common.signOut")}
            </Button>
          </NavbarItem>
        </NavbarContent>
      </Navbar>

      <main className="mx-auto w-full max-w-5xl p-4">
        <Tabs
          aria-label={t("common.sections")}
          selectedKey={route}
          onSelectionChange={(key) => onNavigate(String(key))}
          className="mb-4"
          variant="underlined"
        >
          {TABS.map((tab) => (
            <Tab key={tab} title={t(`layout.tabs.${tab}`)} />
          ))}
        </Tabs>
        {children}
      </main>
    </div>
  );
}
