import { Input } from "@heroui/react";
import { IconAt } from "@tabler/icons-react";
import type { ComponentProps } from "react";
import type { FieldError } from "react-hook-form";

type Props = ComponentProps<typeof Input> & { error?: FieldError };

/// A handle input. The `@` marks it as an identity rather than a username, and
/// autocorrect is off because a handle is a domain, not prose.
export function HandleField({ error, ...props }: Props) {
  return (
    <Input
      variant="bordered"
      autoCapitalize="none"
      autoCorrect="off"
      spellCheck="false"
      isInvalid={Boolean(error)}
      errorMessage={error?.message}
      startContent={
        <IconAt size={16} className="shrink-0 text-default-400" aria-hidden />
      }
      {...props}
    />
  );
}
