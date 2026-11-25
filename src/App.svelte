<script lang="ts">
  import { onMount } from "svelte";
  import {
    api,
    errorText,
    FORMATS,
    type Format,
    type Provider,
    type ProviderInfo,
    type ProviderStatus,
    type QualityReport,
    type Settings,
  } from "./lib/api";
  import ModelPicker from "./lib/ModelPicker.svelte";
  import SettingsDialog from "./lib/SettingsDialog.svelte";

  let catalog = $state<ProviderInfo[]>([]);
  let settings = $state<Settings | null>(null);
  let statuses = $state<ProviderStatus[]>([]);

  let source = $state("");
  let from = $state<Format | "auto">("auto");
  let to = $state<Format>("md");
  let output = $state("");
  let quality = $state<QualityReport | null>(null);
  /** 実際に使った変換元（自動判定の結果を含む）。品質バッジのツールチップに出す */
  let usedFrom = $state<{ format: Format; detected: boolean } | null>(null);

  let starting = $state(true);
  let running = $state(false);
  let jobId = 0;
  let error = $state("");

  /** エラーを画面に出し、ローカルのログにも残す（変換のエラーはバックエンド側で記録済み）。 */
  function fail(context: string, e: unknown, log = true) {
    error = errorText(e);
    if (log) api.logError(context, error);
  }
  let showSettings = $state(false);
  let copied = $state(false);

  const provider = $derived<Provider>(settings?.provider ?? "claude");
  const info = $derived(catalog.find((c) => c.id === provider));
  const status = $derived(statuses.find((s) => s.id === provider));
  const model = $derived(settings?.providers[provider]?.model ?? info?.default_model ?? "");
  const canConvert = $derived(
    !starting && !running && settings !== null && source.trim() !== "" && status?.ok !== false,
  );
  const qualityTitle = $derived.by(() => {
    if (!quality) return "";
    const label = FORMATS.find((f) => f.id === usedFrom?.format)?.label ?? "";
    const head = usedFrom ? `変換元: ${label}${usedFrom.detected ? "（自動判定）" : ""}\n` : "";
    return head + (quality.messages.length ? quality.messages.join("\n") : "問題は見つかりませんでした");
  });

  onMount(async () => {
    try {
      [catalog, settings] = await Promise.all([api.catalog(), api.getSettings()]);
      to = settings.last_to ?? "md";
      from = settings.last_from ?? "auto";
      await refreshStatuses(true);
    } catch (e) {
      fail("startup", e);
    } finally {
      starting = false;
    }
  });

  async function persist(patch: (s: Settings) => void) {
    if (!settings) return;
    patch(settings);
    try {
      settings = await api.saveSettings($state.snapshot(settings) as Settings);
    } catch (e) {
      fail("persist", e);
    }
  }

  function addCustomModel(m: string) {
    persist((s) => {
      const list = s.custom_models[provider] ?? [];
      if (!list.includes(m)) s.custom_models[provider] = [...list, m];
      s.providers[provider].model = m;
    });
  }

  function removeCustomModel(m: string) {
    persist((s) => {
      s.custom_models[provider] = (s.custom_models[provider] ?? []).filter((x) => x !== m);
      if (s.providers[provider].model === m) s.providers[provider].model = info?.default_model ?? "";
    });
  }

  async function convert() {
    if (!canConvert) return;
    error = "";
    running = true;
    const id = ++jobId;
    persist((s) => {
      s.last_to = to;
      s.last_from = from === "auto" ? null : from;
    });
    output = "";
    quality = null;
    try {
      const r = await api.convert(
        {
          job_id: id,
          source,
          from: from === "auto" ? null : from,
          to,
          provider,
          model,
          effort: null,
        },
        (text) => {
          if (id !== jobId) return;
          output += text;
          followOutput();
        },
        () => {
          if (id === jobId) output = "";
        },
      );
      if (id !== jobId) return;
      output = r.output;
      quality = r.quality;
      usedFrom = { format: r.from, detected: r.detected_from };
    } catch (e) {
      if (id === jobId) fail("convert", e, false);
    } finally {
      if (id === jobId) running = false;
    }
  }

  // 生成中は結果欄を末尾に追従させる（ユーザーが上へスクロールしたら追従をやめる）
  let outputEl = $state<HTMLTextAreaElement | null>(null);
  function followOutput() {
    const el = outputEl;
    if (!el) return;
    const nearBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 80;
    if (nearBottom) requestAnimationFrame(() => (el.scrollTop = el.scrollHeight));
  }

  async function cancel() {
    await api.cancel(jobId).catch(() => {});
    jobId++;
    running = false;
  }

  async function copy() {
    try {
      await navigator.clipboard.writeText(output);
      copied = true;
      setTimeout(() => (copied = false), 1200);
    } catch (e) {
      fail("copy", e);
    }
  }

  // ログイン後などに状態を取り直す。ウィンドウに戻ったときに走らせ、連続実行は避ける
  let lastStatusCheck = 0;
  let checkingStatus = false;
  async function refreshStatuses(force = false) {
    if (!settings || checkingStatus || running) return;
    if (!force && Date.now() - lastStatusCheck < 5000) return;
    checkingStatus = true;
    lastStatusCheck = Date.now();
    try {
      statuses = await api.checkProviders();
    } catch (e) {
      fail("status", e);
    } finally {
      checkingStatus = false;
    }
  }

  async function login() {
    try {
      await api.openLogin(provider);
    } catch (e) {
      fail("login", e);
    }
  }

  function onKeydown(e: KeyboardEvent) {
    if ((e.ctrlKey || e.metaKey) && e.key === "Enter") {
      e.preventDefault();
      convert();
    } else if (e.key === "Escape" && running) {
      cancel();
    }
  }
</script>

<svelte:window onkeydown={onKeydown} onfocus={() => refreshStatuses()} />

<div class="app">
  <header>
    <div class="seg" role="tablist" aria-label="プロバイダ">
      {#each catalog as p (p.id)}
        {@const st = statuses.find((s) => s.id === p.id)}
        <button
          role="tab"
          aria-selected={provider === p.id}
          class:active={provider === p.id}
          title={st?.error ?? st?.version ?? ""}
          onclick={() => persist((s) => (s.provider = p.id))}
        >
          <span class="dot" class:ok={st?.ok} class:bad={st && !st.ok}></span>{p.label}
        </button>
      {/each}
    </div>

    {#if info && settings}
      <ModelPicker
        {info}
        {model}
        custom={settings.custom_models[provider] ?? []}
        onselect={(m) => persist((s) => (s.providers[provider].model = m))}
        onadd={addCustomModel}
        onremove={removeCustomModel}
      />
    {/if}

    <div class="spacer"></div>
    <button class="ghost icon" onclick={() => (showSettings = true)} title="設定" aria-label="設定">⚙</button>
  </header>

  <main>
    <section class="pane">
      <div class="pane-head">
        <select bind:value={from} aria-label="変換元">
          <option value="auto">自動判定</option>
          {#each FORMATS as f (f.id)}
            <option value={f.id}>{f.label}</option>
          {/each}
        </select>
      </div>
      <textarea bind:value={source} placeholder="ここに貼り付け（Ctrl+Enter で変換）" spellcheck="false"></textarea>
    </section>

    <div class="middle">
      {#if running}
        <button class="primary" onclick={cancel} title="Esc">
          <span class="spinner" aria-hidden="true"></span>中止
        </button>
      {:else}
        <button class="primary" onclick={convert} disabled={!canConvert} title="Ctrl+Enter">
          {starting ? "起動準備中…" : "変換 →"}
        </button>
      {/if}
    </div>

    <section class="pane">
      <div class="progress" class:on={running} aria-hidden="true"></div>
      <div class="pane-head">
        <div class="seg" role="tablist" aria-label="変換先">
          {#each FORMATS as f (f.id)}
            <button role="tab" aria-selected={to === f.id} class:active={to === f.id} onclick={() => (to = f.id)}>
              {f.short}
            </button>
          {/each}
        </div>
        <div class="spacer"></div>
        {#if quality && !running}
          <span class="badge {quality.grade}" title={qualityTitle}>{quality.grade.toUpperCase()}</span>
        {/if}
        <button onclick={copy} disabled={!output || running}>{copied ? "✓" : "コピー"}</button>
      </div>
      <textarea
        bind:this={outputEl}
        bind:value={output}
        readonly={running}
        aria-busy={running}
        placeholder={running ? "変換中…" : ""}
        spellcheck="false"></textarea>
    </section>
  </main>

  {#if error || status?.ok === false}
    <div class="error" role="alert">
      <span>{error || status?.error}</span>
      {#if status?.auth?.state === "logged_out" || /ログイン/.test(error)}
        <button class="small" onclick={login}>ログイン</button>
      {/if}
    </div>
  {/if}
</div>

{#if showSettings && settings}
  <SettingsDialog
    {settings}
    {catalog}
    {statuses}
    onclose={() => (showSettings = false)}
    onsave={async (s) => {
      try {
        settings = await api.saveSettings(s);
        showSettings = false;
        await refreshStatuses(true);
      } catch (e) {
        fail("settings", e);
      }
    }}
  />
{/if}
