import { Input } from "@heroui/react";
import type { ComponentProps } from "react";
import type { FieldError } from "react-hook-form";

type Props = ComponentProps<typeof Input> & { error?: FieldError };

export function Field({ error, ...props }: Props) {
  return (
    <Input
      variant="bordered"
      isInvalid={Boolean(error)}
      errorMessage={error?.message}
      {...props}
    />
  );
}
