import { convert } from "@asciidoctor/core";

self.addEventListener("message", async ({ data: source }: MessageEvent<string>) => {
  try {
    const html = await convert(source, { safe: "secure", standalone: false });
    self.postMessage(typeof html === "string" ? html : null);
  } catch {
    self.postMessage(null);
  }
});
