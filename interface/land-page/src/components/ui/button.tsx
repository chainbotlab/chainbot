import * as React from "react";
import { cva, type VariantProps } from "class-variance-authority";

import { cn } from "@/lib/utils";

const buttonVariants = cva(
  "inline-flex items-center justify-center rounded-full text-sm font-medium transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary/40 disabled:pointer-events-none disabled:opacity-50",
  {
    variants: {
      variant: {
        primary: "bg-primary text-primary-foreground hover:bg-primary/90",
        secondary: "bg-secondary text-secondary-foreground hover:bg-secondary/80",
        ghost: "bg-transparent text-foreground hover:bg-secondary/60",
      },
      size: {
        md: "h-11 px-5",
        lg: "h-12 px-6",
      },
    },
    defaultVariants: {
      variant: "primary",
      size: "md",
    },
  },
);

type LinkButtonProps = React.AnchorHTMLAttributes<HTMLAnchorElement> &
  VariantProps<typeof buttonVariants> & {
    href: string;
  };

type ActionButtonProps = React.ButtonHTMLAttributes<HTMLButtonElement> &
  VariantProps<typeof buttonVariants> & {
    href?: undefined;
  };

export function Button(props: LinkButtonProps | ActionButtonProps) {
  const { className, variant, size } = props;

  if ("href" in props && props.href) {
    const { href, ...anchorProps } = props;
    return (
      <a
        className={cn(buttonVariants({ variant, size }), className)}
        href={href}
        {...anchorProps}
      />
    );
  }

  const buttonProps = props as ActionButtonProps;

  return (
    <button
      className={cn(buttonVariants({ variant, size }), className)}
      {...buttonProps}
    />
  );
}

export { buttonVariants };
