import { clsx, type ClassValue } from "clsx";
import { twMerge } from "tailwind-merge";

/**
 * Combines Tailwind classes cleanly.
 * `clsx` handles conditional classes; `tailwind-merge` removes duplicates.
 */
export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs));
}
