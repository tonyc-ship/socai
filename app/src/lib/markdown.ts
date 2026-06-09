import { marked } from "marked";
import DOMPurify from "dompurify";

// Markdown → sanitized HTML for agent final answers. `marked` handles GFM
// (tables, strikethrough, task lists, autolinks, fenced code, …) and
// `DOMPurify` strips anything unsafe from the result before it reaches the DOM.

marked.setOptions({ gfm: true, breaks: true });

// Open links in a new tab/window and harden against reverse-tabnabbing. Runs
// after sanitization so the attributes we add are not themselves stripped.
DOMPurify.addHook("afterSanitizeAttributes", (node) => {
  if (node.tagName === "A") {
    node.setAttribute("target", "_blank");
    node.setAttribute("rel", "noopener noreferrer");
  }
});

export function renderMarkdown(src: string): string {
  const html = marked.parse(src, { async: false });
  return DOMPurify.sanitize(html);
}
