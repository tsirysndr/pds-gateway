import { Select, SelectItem } from "@heroui/react";
import { IconLanguage } from "@tabler/icons-react";
import { useAtom } from "jotai";
import { useTranslation } from "react-i18next";
import { languageAtom } from "../atoms/store";
import { detectLanguage, languages } from "../i18n";

/// Switches the console's language, and remembers the choice per browser.
export function LanguageSelect() {
  const [stored, setStored] = useAtom(languageAtom);
  const { i18n, t } = useTranslation();
  const current = detectLanguage(stored || null, navigator.languages ?? []);

  return (
    <Select
      aria-label={t("common.language")}
      size="sm"
      variant="bordered"
      className="w-48 min-w-48"
      selectedKeys={[current]}
      startContent={<IconLanguage size={16} className="text-default-400" aria-hidden />}
      onSelectionChange={(keys) => {
        const key = String(Array.from(keys)[0] ?? "");
        if (!key) return;
        setStored(key);
        void i18n.changeLanguage(key);
      }}
    >
      {languages.map((language) => (
        <SelectItem key={language.key}>{language.label}</SelectItem>
      ))}
    </Select>
  );
}
