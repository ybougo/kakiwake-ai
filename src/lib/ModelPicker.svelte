<script lang="ts">
  import type { ProviderInfo } from "./api";

  interface Props {
    info: ProviderInfo;
    model: string;
    custom: string[];
    onselect: (model: string) => void;
    onadd: (model: string) => void;
    onremove: (model: string) => void;
  }
  let { info, model, custom, onselect, onadd, onremove }: Props = $props();

  const ADD = "__add__";
  let adding = $state(false);
  let draft = $state("");
  let input = $state<HTMLInputElement | null>(null);

  const isPreset = $derived(info.presets.some((p) => p.id === model));
  const isCustom = $derived(custom.includes(model));
  // 設定ファイルなどで候補にないモデルが入っているときも選択肢に出す
  const isOrphan = $derived(!!model && !isPreset && !isCustom);

  function onchange(e: Event) {
    const el = e.currentTarget as HTMLSelectElement;
    if (el.value === ADD) {
      el.value = model;
      draft = "";
      adding = true;
      queueMicrotask(() => input?.focus());
      return;
    }
    onselect(el.value);
  }

  function commit() {
    const id = draft.trim();
    if (id) onadd(id);
    adding = false;
  }

  function onkey(e: KeyboardEvent) {
    if (e.key === "Enter") {
      e.preventDefault();
      e.stopPropagation();
      commit();
    } else if (e.key === "Escape") {
      e.stopPropagation();
      adding = false;
    }
  }
</script>

<div class="model-picker">
  {#if adding}
    <input
      bind:this={input}
      bind:value={draft}
      onkeydown={onkey}
      onblur={() => (adding = false)}
      placeholder="モデル ID を入力して Enter"
      aria-label="新しいモデル ID"
      spellcheck="false"
    />
  {:else}
    <select value={model} {onchange} aria-label="モデル" title={model}>
      {#each info.presets as p (p.id)}
        <option value={p.id}>{p.label}{p.note ? `（${p.note}）` : ""}</option>
      {/each}
      {#each custom as m (m)}
        <option value={m}>{m}</option>
      {/each}
      {#if isOrphan}
        <option value={model}>{model}</option>
      {/if}
      <option value={ADD}>＋ モデル ID を追加…</option>
    </select>
    {#if isCustom}
      <button class="ghost small" title="候補から削除" aria-label="候補から削除" onclick={() => onremove(model)}>
        ×
      </button>
    {/if}
  {/if}
</div>
