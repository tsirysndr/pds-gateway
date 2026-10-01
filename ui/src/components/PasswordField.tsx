import { useState } from "react";
import { Button, Input } from "@heroui/react";
import { IconEye, IconEyeOff, IconLock } from "@tabler/icons-react";
import type { ComponentProps } from "react";
import type { FieldError } from "react-hook-form";
import { useTranslation } from "react-i18next";

type Props = Omit<ComponentProps<typeof Input>, "type"> & { error?: FieldError };

/// A password input with a lock and a reveal toggle.
///
/// Revealing is a deliberate action and resets on every mount, so a password is
/// never left visible on a screen the owner walked away from.
export function PasswordField({ error, ...props }: Props) {
  const [visible, setVisible] = useState(false);
  const { t } = useTranslation();

  return (
    <Input
      variant="bordered"
      type={visible ? "text" : "password"}
      isInvalid={Boolean(error)}
      errorMessage={error?.message}
      startContent={
        <IconLock size={16} className="shrink-0 text-default-400" aria-hidden />
      }
      endContent={
        <Button
          isIconOnly
          size="sm"
          variant="light"
          tabIndex={-1}
          aria-label={visible ? t("common.hidePassword") : t("common.showPassword")}
          onPress={() => setVisible((shown) => !shown)}
        >
          {visible ? (
            <IconEyeOff size={16} className="text-default-500" aria-hidden />
          ) : (
            <IconEye size={16} className="text-default-500" aria-hidden />
          )}
        </Button>
      }
      {...props}
    />
  );
}
