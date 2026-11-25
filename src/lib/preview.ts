import type { Format } from "./api";

// リンク先や外部リソースを持ち込まず、文書の構造だけをアプリ内に表示する。
const previewTags = [
  "a",
  "abbr",
  "b",
  "blockquote",
  "br",
  "caption",
  "code",
  "col",
  "colgroup",
  "dd",
  "del",
  "details",
  "div",
  "dl",
  "dt",
  "em",
  "figcaption",
  "figure",
  "h1",
  "h2",
  "h3",
  "h4",
  "h5",
  "h6",
  "hr",
  "i",
  "img",
  "input",
  "kbd",
  "li",
  "mark",
  "ol",
  "p",
  "pre",
  "s",
  "small",
  "span",
  "strong",
  "sub",
  "summary",
  "sup",
  "table",
  "tbody",
  "td",
  "tfoot",
  "th",
  "thead",
  "tr",
  "u",
  "ul",
];

const ASCII_DOC_TIMEOUT_MS = 10_000;

async function renderAsciiDoc(source: string, signal: AbortSignal): Promise<string> {
  const { default: AsciiDocWorker } = await import("./asciidoc.worker?worker&inline");
  const stop = AbortSignal.any([signal, AbortSignal.timeout(ASCII_DOC_TIMEOUT_MS)]);
  stop.throwIfAborted();
  const worker = new AsciiDocWorker();
  const listeners = new AbortController();
  try {
    return await new Promise((resolve, reject) => {
      stop.addEventListener("abort", () => reject(stop.reason), { signal: listeners.signal });
      worker.onmessage = ({ data }: MessageEvent<string | null>) => {
        if (data === null) reject(new Error("AsciiDoc preview did not return HTML"));
        else resolve(data);
      };
      worker.onerror = (event) => reject(new Error(event.message));
      worker.postMessage(source);
    });
  } finally {
    listeners.abort();
    worker.terminate();
  }
}

async function renderMarkup(format: Exclude<Format, "plain">, source: string, signal: AbortSignal): Promise<string> {
  switch (format) {
    case "md": {
      const { marked } = await import("marked");
      return marked.parse(source, { gfm: true, async: false });
    }
    case "adoc":
      return renderAsciiDoc(source, signal);
    case "textile": {
      const { default: textile } = await import("textile-js");
      return textile(source);
    }
  }
}

export async function renderPreview(format: Format, source: string, signal: AbortSignal): Promise<DocumentFragment> {
  if (format === "plain") {
    const text = document.createElement("pre");
    text.textContent = source;
    const fragment = document.createDocumentFragment();
    fragment.append(text);
    return fragment;
  }

  const [html, { default: DOMPurify }] = await Promise.all([renderMarkup(format, source, signal), import("dompurify")]);
  signal.throwIfAborted();
  const fragment = DOMPurify.sanitize(html, {
    ALLOWED_TAGS: previewTags,
    ALLOWED_ATTR: ["align", "alt", "checked", "colspan", "rowspan", "span", "start", "title", "type", "value"],
    ALLOW_ARIA_ATTR: false,
    ALLOW_DATA_ATTR: false,
    RETURN_DOM_FRAGMENT: true,
  });

  for (const image of fragment.querySelectorAll("img")) {
    image.replaceWith(image.alt);
  }
  for (const input of fragment.querySelectorAll("input")) {
    if (input.type === "checkbox") input.disabled = true;
    else input.remove();
  }
  return fragment;
}
