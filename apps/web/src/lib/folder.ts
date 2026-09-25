/**
 * Ask the OS for a directory, when running inside the desktop shell.
 *
 * The sibling of `pickFile` in `imports.ts`, and separate from it because a directory dialog is a
 * different call with a different result: `open({ directory: true })` returns a folder path, and a
 * user who picked a file when a folder was wanted gets a refusal from the daemon rather than a
 * dialog that would not have let them.
 *
 * Resolves to `null` outside the shell or when the user cancels — both are "no folder", and the
 * caller should not have to tell them apart. The plugin is imported lazily so a browser build never
 * pulls it in.
 */
export async function pickFolder(title = "Choose a folder"): Promise<string | null> {
  const tauri = (globalThis as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__;
  if (!tauri) return null;
  try {
    const { open } = await import("@tauri-apps/plugin-dialog");
    const chosen = await open({ directory: true, multiple: false, title });
    return typeof chosen === "string" ? chosen : null;
  } catch {
    // A missing plugin should degrade to the typed-path field, not break the screen.
    return null;
  }
}
