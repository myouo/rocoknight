import * as React from "react";
import { cn } from "../../lib/utils";

export interface ButtonProps extends React.ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: "primary" | "ghost";
}

export const Button = React.forwardRef<HTMLButtonElement, ButtonProps>(
  ({ className, variant = "primary", ...props }, ref) => (
    <button
      ref={ref}
      className={cn(
        "inline-flex items-center justify-center rounded-md text-sm font-semibold transition-colors",
        "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-white/60 disabled:opacity-50",
        variant === "primary" && "bg-amber-500 text-black hover:bg-amber-400",
        variant === "ghost" && "bg-transparent text-white/80 hover:bg-white/10",
        className
      )}
      {...props}
    />
  )
);
Button.displayName = "Button";
