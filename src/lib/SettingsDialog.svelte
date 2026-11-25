<script lang="ts">
  import { onMount, untrack } from "svelte";
  import { FORMATS, type Format, type ProviderInfo, type ProviderStatus, type Settings } from "./api";

  interface Props {
    settings: Settings;
    catalog: ProviderInfo[];
    statuses: ProviderStatus[];
    onclose: () => void;
    onsave: (s: Settings) => void;
  }
  let { settings, catalog, statuses, onclose, onsave }: Props = $props();

  // 編集用のコピー（保存するまで本体に反映しない）
  let draft = $state<Settings>(untrack(() => structuredClone($state.snapshot(settings)) as Settings));
  let tab = $state<"providers" | "convert" | "prompts">("providers");
  let dialog: HTMLDialogElement;

  onMount(() => dialog.showModal());

  function removeCustom(p: ProviderInfo["id"], m: string) {
    draft.custom_models[p] = (draft.custom_models[p] ?? []).filter((x) => x !== m);
  }

  function resetModel(p: ProviderInfo) {
    draft.providers[p.id].model = p.default_model;
  }

  const extraFor = (f: Format) => draft.extra_instructions[f] ?? "";
  const setExtra = (f: Format, v: string) => (draft.extra_instructions[f] = v);
</script>

<dialog bind:this={dialog} {onclose} class="settings">
  <form
    method="dialog"
    onsubmit={(e) => {
      e.preventDefault();
      onsave($state.snapshot(draft) as Settings);
    }}
  >
    <header>
      <h2>設定</h2>
      <div class="seg">
        <button type="button" class:active={tab === "providers"} onclick={() => (tab = "providers")}>プロバイダ</button>
        <button type="button" class:active={tab === "convert"} onclick={() => (tab = "convert")}>変換・品質</button>
        <button type="button" class:active={tab === "prompts"} onclick={() => (tab = "prompts")}>追加指示</button>
      </div>
    </header>

    <div class="body">
      {#if tab === "providers"}
        {#each catalog as p (p.id)}
          {@const st = statuses.find((s) => s.id === p.id)}
          <fieldset>
            <legend>
              <span class="dot" class:ok={st?.ok} class:bad={st && !st.ok}></span>
              {p.label}
              <span class="muted">{st?.ok ? st.version : (st?.error ?? "")}</span>
            </legend>
            <label>
              <span>実行ファイル</span>
              <input
                bind:value={draft.providers[p.id].path}
                placeholder={st?.path ?? `${p.id}（PATH から自動検出）`}
                spellcheck="false"
              />
            </label>
            <label>
              <span>モデル ID</span>
              <div class="row">
                <input bind:value={draft.providers[p.id].model} list={`models-${p.id}`} spellcheck="false" />
                <button type="button" class="ghost small" onclick={() => resetModel(p)}>既定に戻す</button>
              </div>
              <datalist id={`models-${p.id}`}>
                {#each p.presets as m (m.id)}<option value={m.id}>{m.label}</option>{/each}
                {#each draft.custom_models[p.id] ?? [] as m (m)}<option value={m}></option>{/each}
              </datalist>
            </label>
            {#if (draft.custom_models[p.id] ?? []).length}
              <div class="chips">
                <span class="muted">追加したモデル:</span>
                {#each draft.custom_models[p.id] ?? [] as m (m)}
                  <span class="chip">
                    {m}<button type="button" title="削除" onclick={() => removeCustom(p.id, m)}>×</button>
                  </span>
                {/each}
              </div>
            {/if}
            <label>
              <span>推論量</span>
              <select bind:value={draft.providers[p.id].effort}>
                {#each p.efforts as e (e)}<option value={e}>{e === "" ? "CLI の既定" : e}</option>{/each}
              </select>
            </label>
            <label>
              <span>追加引数</span>
              <input
                bind:value={draft.providers[p.id].extra_args}
                placeholder="空白区切り（上級者向け）"
                spellcheck="false"
              />
            </label>
          </fieldset>
        {/each}
      {:else if tab === "convert"}
        <label class="check">
          <input type="checkbox" bind:checked={draft.quality_check} />
          変換後に品質チェックを行う（ローカルで実行。外部送信なし）
        </label>
        <label class="check">
          <input type="checkbox" bind:checked={draft.retry_on_ng} />
          品質チェックが NG のとき 1 回だけ自動で再変換する
        </label>
        <label>
          <span>タイムアウト（秒）</span>
          <input type="number" min="10" max="1800" bind:value={draft.timeout_sec} />
        </label>
        <p class="muted">
          入力は選択中の LLM CLI にのみ送信されます。CLI の自動更新・テレメトリ・MCP・Web 検索は無効化して起動します。
        </p>
      {:else}
        <p class="muted">変換先ごとに、プロンプトへ追加する指示を書けます（例: 「見出しの番号は削除する」）。</p>
        {#each FORMATS as f (f.id)}
          <label>
            <span>{f.label}</span>
            <textarea
              rows="2"
              value={extraFor(f.id)}
              oninput={(e) => setExtra(f.id, (e.currentTarget as HTMLTextAreaElement).value)}></textarea>
          </label>
        {/each}
      {/if}
    </div>

    <footer>
      <button type="button" class="ghost" onclick={() => dialog.close()}>キャンセル</button>
      <button type="submit" class="primary">保存</button>
    </footer>
  </form>
</dialog>
