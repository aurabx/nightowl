import type { ReactNode } from "react";

interface FieldProps {
  label: string;
  hint?: string;
  error?: string;
  children: ReactNode;
}

/**
 * Form field wrapper with a label, optional hint, and inline validation
 * error. Renders any input (text / number / select) as `children` so we
 * keep the visual styling consistent across the app without coupling to
 * a specific input type.
 */
export function Field({ label, hint, error, children }: FieldProps) {
  return (
    <label className="block">
      <span className="block text-sm font-medium text-slate-200">{label}</span>
      {hint && !error && (
        <span className="mt-0.5 block text-xs text-slate-500">{hint}</span>
      )}
      {error && (
        <span className="mt-0.5 block text-xs text-red-400">{error}</span>
      )}
      <div className="mt-1.5">{children}</div>
    </label>
  );
}
