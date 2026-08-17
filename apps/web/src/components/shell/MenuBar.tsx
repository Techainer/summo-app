import * as DropdownMenu from "@radix-ui/react-dropdown-menu";

import { useT } from "../../i18n/context";
import { cn } from "../../lib/cn";
import { MENU } from "../../lib/menu";
import { isMac } from "../../lib/shell";

/**
 * The menu bar, for the platforms whose menu bar lives in the window.
 *
 * Not on macOS: the system draws one at the top of the screen from
 * `apps/desktop/src-tauri/src/main.rs`, and a second bar inside the window would be a copy of it.
 * Windows and Linux hang the menu off the window frame, and this window is `decorations: false` —
 * there is no frame, so the native menu is built and never appears. Without this, everything in it
 * was unreachable on two of the three platforms the app ships on.
 *
 * The items come from `lib/menu.ts`, which is also what the native bar is built from, so the two
 * cannot quietly disagree about what the app can do.
 */
export function MenuBar({ onChoose }: { onChoose: (id: string) => void }) {
  const t = useT();
  const mod = isMac() ? "⌘" : "Ctrl";

  return (
    <nav aria-label={t("menu.bar")} className="hidden items-center gap-0.5 md:flex">
      {MENU.map((group) => (
        <DropdownMenu.Root key={group.labelKey}>
          <DropdownMenu.Trigger
            className={cn(
              "text-meta rounded-md px-2 py-1 transition-colors",
              "hover:bg-bg-soft data-[state=open]:bg-bg-soft text-fg-dim hover:text-fg",
            )}
          >
            {t(group.labelKey)}
          </DropdownMenu.Trigger>
          <DropdownMenu.Portal>
            <DropdownMenu.Content
              sideOffset={4}
              align="start"
              className="border-line bg-bg-elevated z-50 min-w-56 rounded-[var(--radius-card)] border p-1 shadow-[var(--shadow-pop)]"
            >
              {group.items.map((item, at) =>
                item === "separator" ? (
                  <DropdownMenu.Separator key={at} className="bg-line my-1 h-px" />
                ) : (
                  <DropdownMenu.Item
                    key={item.id}
                    onSelect={() => onChoose(item.id)}
                    className="text-body data-[highlighted]:bg-bg-soft flex cursor-pointer items-center justify-between gap-6 rounded-md px-2 py-1.5 outline-none"
                  >
                    {t(item.labelKey)}
                    {item.keys && (
                      <span className="text-fg-faint text-micro tabular">
                        {item.keys.map((key) => (key === "mod" ? mod : key)).join("+")}
                      </span>
                    )}
                  </DropdownMenu.Item>
                ),
              )}
            </DropdownMenu.Content>
          </DropdownMenu.Portal>
        </DropdownMenu.Root>
      ))}
    </nav>
  );
}
