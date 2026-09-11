import { cva, type VariantProps } from "class-variance-authority";
import type { ButtonHTMLAttributes } from "react";
import { cn } from "@/lib/utils";

const buttonVariants = cva(
  "inline-flex items-center justify-center rounded-md bg-teal-300 px-3 py-2 text-sm font-semibold text-slate-950 transition-colors hover:bg-teal-200 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-teal-200 disabled:pointer-events-none disabled:opacity-50",
  {
    variants: { size: { default: "", compact: "px-2 py-1 text-xs" } },
    defaultVariants: { size: "default" },
  },
);

type ButtonProps = ButtonHTMLAttributes<HTMLButtonElement> &
  VariantProps<typeof buttonVariants>;

export function Button({ className, size, ...props }: ButtonProps) {
  return (
    <button
      className={cn(buttonVariants({ size }), className)}
      type="button"
      {...props}
    />
  );
}
