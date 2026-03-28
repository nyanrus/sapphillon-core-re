/**
 * Sapphillon JS engine plugin — Extism JS PDK (QuickJS / eval mode).
 *
 * Compiled to a WASM module via `extism-js`:
 *   npm install -g @extism/js-pdk
 *   extism-js plugin.js -o js_engine.wasm
 *
 * Input  (JSON):  { script: string, pre_scripts: string[] }
 * Output (string): captured stdout, lines joined by '\n'
 *
 * Host function used:
 *   dispatch(JSON { op_key, args_json }) -> JSON { ok: string } | { err: string }
 *   Provided by sapphillon_deno (Rust/Extism host) and ultimately resolved by
 *   PluginDispatcher → sapphillon_core.
 */

function run_script() {
    const input = JSON.parse(Host.inputString());
    const script     = input.script      ?? '';
    const preScripts = input.pre_scripts ?? [];

    // ── Stdout capture ──────────────────────────────────────────────────────
    const stdoutBuf = [];
    const _put = (text) => stdoutBuf.push(text);
    const _fmt = (...a) =>
        a.map(x => (typeof x === 'object' ? JSON.stringify(x) : String(x))).join(' ');

    globalThis.console = {
        log:   (...a) => _put(_fmt(...a)),
        info:  (...a) => _put('[INFO] '  + _fmt(...a)),
        warn:  (...a) => _put('[WARN] '  + _fmt(...a)),
        error: (...a) => _put('[ERROR] ' + _fmt(...a)),
        debug: (...a) => _put('[DEBUG] ' + _fmt(...a)),
    };

    // ── Dispatch bridge ─────────────────────────────────────────────────────
    // Each call reaches the Rust PluginDispatcher via Extism host function.
    globalThis.__sapphillon_dispatch = function(opKey, argsJson) {
        const raw      = Host.call('dispatch', JSON.stringify({ op_key: opKey, args_json: argsJson }));
        const envelope = JSON.parse(raw);
        if (envelope.err) throw new Error(envelope.err);
        return envelope.ok;
    };

    // ── Pre-scripts (plugin shims injected by sapphillon_core) ──────────────
    for (const src of preScripts) {
        eval(src);  // eslint-disable-line no-eval
    }

    // ── Main workflow ───────────────────────────────────────────────────────
    eval(script);  // eslint-disable-line no-eval

    Host.outputString(stdoutBuf.join('\n'));
}

module.exports = { run_script };
