import { ChevronDown } from "lucide-react";
import type { ReactNode, SelectHTMLAttributes } from "react";

interface SelectProps
  extends Omit<SelectHTMLAttributes<HTMLSelectElement>, "className"> {
  /** Optional extra classes appended to the wrapper, e.g. width. */
  className?: string;
  children: ReactNode;
}

/**
 * Dark-themed dropdown that matches the rest of the form chrome.
 *
 * Native `<select>` widgets ignore most CSS on macOS, so we hide the
 * native arrow with `appearance-none` and overlay our own
 * `ChevronDown` icon. The option list rendered when the user clicks
 * is still the OS-native one (browsers cannot style that), which is
 * fine — we're only fixing the resting appearance.
 */
export function Select({ className = "", children, ...rest }: SelectProps) {
  return (
    <div className={`relative inline-block ${className}`}>
      <select
        {...rest}
        className={
          // Match the search input chrome: same border, bg, text, padding,
          // focus outline. Extra right padding leaves room for the chevron.
          "w-full appearance-none rounded border border-slate-700 bg-slate-900 " +
          "py-1 pl-2 pr-7 text-sm text-slate-200 " +
          "focus:border-sky-500 focus:outline-none cursor-pointer"
        }
      >
        {children}
      </select>
      <ChevronDown
        className="pointer-events-none absolute right-1.5 top-1/2 size-3.5 -translate-y-1/2 text-slate-500"
        aria-hidden
      />
    </div>
  );
}
