<script lang="ts">
  import type { Format } from "./api";
  import { renderPreview } from "./preview";

  let { format, source }: { format: Format; source: string } = $props();
  let content: HTMLDivElement | undefined = $state();
  let loading = $state(true);
  let errorMessage = $state("");

  $effect(() => {
    const target = content;
    if (!target) return;
    const controller = new AbortController();
    loading = true;
    errorMessage = "";
    target.replaceChildren();

    void renderPreview(format, source, controller.signal).then(
      (fragment) => {
        if (controller.signal.aborted) return;
        target.replaceChildren(fragment);
        loading = false;
      },
      (error: unknown) => {
        if (controller.signal.aborted) return;
        errorMessage =
          error instanceof DOMException && error.name === "TimeoutError"
            ? "AsciiDoc のプレビューがタイムアウトしました。"
            : "プレビューを表示できませんでした。";
        loading = false;
      },
    );
    return () => controller.abort();
  });
</script>

<!-- svelte-ignore a11y_no_noninteractive_tabindex (スクロール領域をキーボードで操作できるようにする) -->
<div class="preview" role="region" aria-label="変換結果のプレビュー" aria-busy={loading} tabindex="0">
  {#if loading}
    <p class="muted" role="status">プレビューを表示中…</p>
  {:else if errorMessage}
    <p role="alert">{errorMessage}原文に戻るには「プレビュー」をもう一度押してください。</p>
  {/if}
  <div bind:this={content} class:plain={format === "plain"}></div>
</div>
