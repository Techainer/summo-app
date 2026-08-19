import { useNavigate, useRouterState } from "@tanstack/react-router";
import { useCallback, useMemo } from "react";

import { Library } from "../components/Library";
import { NoteClient } from "../lib/notes";
import { useEngine } from "../lib/engine-context";
import { useT } from "../i18n/context";
import type { LibrarySearch } from "../router";

/**
 * The library, with its filters in the URL.
 *
 * Every filter lives here rather than inside `Library` because the app's own sidebar sets the
 * folder too, and a filter with two owners shows whichever was told last — picking a folder in the
 * sidebar used to navigate here and then list the whole vault. Putting all four in one place makes
 * that impossible rather than merely fixed, and the filtered view becomes something you can
 * reload, share and step back out of, which is what the route already claimed.
 */
export function LibraryScreen() {
  const { library, start, handshake } = useEngine();
  const navigate = useNavigate();
  const t = useT();
  const search = useRouterState({
    select: (s) => s.location.search,
  });

  const patch = useCallback(
    (next: Partial<LibrarySearch>) => {
      void navigate({
        to: "/library",
        search: (old: LibrarySearch) => ({ ...old, ...next }),
      });
    },
    [navigate],
  );

  /// Writing, from the shelf that holds what is written.
  ///
  /// A blank note and then straight into it, rather than a screen that lists notes and waits: the
  /// person pressed a button that says "write a note", and the next thing they should see is
  /// somewhere to type.
  const write = useCallback(async () => {
    const client = new NoteClient(handshake);
    const { id } = await client.create(t("notes.untitled"), "");
    await navigate({ to: "/pages/$pageId", params: { pageId: id } });
  }, [handshake, navigate, t]);

  // A new array identity on every render would refetch the library on every render; the string is
  // what actually changed.
  const tags = useMemo(() => (search.tag ?? "").split(",").filter(Boolean), [search.tag]);

  return (
    <Library
      client={library}
      folder={search.folder}
      tags={tags}
      colour={search.colour}
      query={search.q ?? ""}
      onFolder={(folder) => patch({ folder })}
      // Joined here because that is how it travels — both in this URL and in the daemon's query
      // string, which cannot express a repeated key every deserialiser agrees on.
      onTags={(next) => patch({ tag: next.length > 0 ? next.join(",") : undefined })}
      onColour={(colour) => patch({ colour })}
      onQuery={(q) => patch({ q: q === "" ? undefined : q })}
      onRecord={() => {
        void navigate({ to: "/" });
        void start();
      }}
      onWrite={() => void write()}
      onOpen={(id) => void navigate({ to: "/pages/$pageId", params: { pageId: id } })}
    />
  );
}
