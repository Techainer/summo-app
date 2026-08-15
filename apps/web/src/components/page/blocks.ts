import Image, { type ImageOptions } from "@tiptap/extension-image";
import Link from "@tiptap/extension-link";
import { Table, TableCell, TableHeader, TableRow } from "@tiptap/extension-table";

/**
 * The nodes the rich editor adds on top of the starter kit, and the reasons each one is bent.
 *
 * Every extension here is adjusted to fit a file rather than a database. That is the whole theme:
 * the vault is Markdown a person opens in Obsidian, so a feature that cannot be written down in
 * Markdown is a feature that disappears on save — and disappearing on save is the failure mode this
 * editor is built to make impossible.
 */

/** The class a column's alignment is drawn with, and the value it came from. */
const ALIGNMENT = {
  left: "text-left",
  center: "text-center",
  right: "text-right",
} as const;

/**
 * A column's alignment, kept on the cell.
 *
 * ProseMirror tables have no column objects — only rows of cells — while GFM writes alignment once,
 * in the divider row. So the converter puts the same value on every cell of a column and reads it
 * back off the header, and this is the attribute it puts it in. Without it, a table written
 * `| :---: |` would come back `| --- |` and every note containing one would open in the plain
 * textarea instead.
 */
const align = {
  align: {
    default: null as string | null,
    // Read from the class rather than from an inline style, and written the same way. An inline
    // style would be the obvious choice and cannot be used: the stylesheet test scans this source
    // for utility names and a template string spelling `text-align` reads to it as a utility called
    // `align` that generates nothing. Classes are also what the rest of this interface uses, so the
    // alignment a table renders with comes out of the same place as everything else's.
    parseHTML: (element: HTMLElement) =>
      Object.entries(ALIGNMENT).find(([, css]) => element.classList.contains(css))?.[0] ??
      element.style.textAlign ??
      null,
    renderHTML: (attributes: { align?: string | null }) => {
      const css = ALIGNMENT[attributes.align as keyof typeof ALIGNMENT];
      return css ? { class: css } : {};
    },
  },
};

const AlignedCell = TableCell.extend({
  addAttributes() {
    return { ...this.parent?.(), ...align };
  },
});

const AlignedHeader = TableHeader.extend({
  addAttributes() {
    return { ...this.parent?.(), ...align };
  },
});

/**
 * A picture, stored in the vault and addressed by the daemon.
 *
 * The document holds `attachments/<name>` — a path relative to the vault root, because that is what
 * makes the note render in Obsidian as well as here, and because a `http://127.0.0.1:<port>` in a
 * saved file would rot the moment the daemon restarted on a different port. A browser needs an
 * address, so the translation happens at the last possible moment: on render, and nowhere else.
 *
 * `allowBase64` is off deliberately. A data URI would put a 400 kB screenshot into the Markdown as
 * half a megabyte of base64 on one unwrappable line, in a file the user greps.
 */
interface VaultImageOptions extends ImageOptions {
  /** A vault link to something a browser can fetch. */
  resolve: (link: string) => string;
}

export const VaultImage = Image.extend<VaultImageOptions>({
  addOptions() {
    return { ...(this.parent?.() as ImageOptions), resolve: (link: string) => link };
  },
  renderHTML({ HTMLAttributes }) {
    const src = String(HTMLAttributes.src ?? "");
    return [
      "img",
      {
        ...HTMLAttributes,
        // An absolute URL is left alone: a note may legitimately point at a picture on the web, and
        // rewriting that through the daemon would ask it to proxy the internet.
        src: /^[a-z]+:/i.test(src) ? src : this.options.resolve(src),
        // A picture in a note is content, and content that has finished loading should not push the
        // paragraph under it down the page.
        loading: "lazy",
      },
    ];
  },
});

/**
 * Links, with a sub-page marked as one.
 *
 * A sub-page is an ordinary Markdown link to `/pages/<id>` — which is what keeps it a link in every
 * other tool, and what stops this being a shape only Summo can read. The attribute is so the
 * stylesheet can draw it as a page rather than as a URL; nothing about the document depends on it.
 */
export const PageLink = Link.extend({
  renderHTML({ HTMLAttributes }) {
    const href = String(HTMLAttributes.href ?? "");
    return [
      "a",
      { ...HTMLAttributes, ...(href.startsWith("/pages/") ? { "data-page": "" } : {}) },
      0,
    ];
  },
});

/**
 * Tables.
 *
 * Not resizable, and that is a decision rather than an omission. Column widths live in the
 * ProseMirror document and have nowhere to go in Markdown, so a width somebody dragged would be
 * written to a file that cannot hold it and be gone on the next open — a control that looks like it
 * works and does not. Better to not offer it.
 */
export const TABLE = [Table.configure({ resizable: false }), TableRow, AlignedHeader, AlignedCell];
