#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::io::Read as _;

use serde::Deserialize;
use tao::{
    dpi::{LogicalSize, PhysicalPosition},
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoop},
    window::WindowBuilder,
};
#[cfg(target_os = "windows")]
use tao::platform::windows::{WindowBuilderExtWindows, WindowExtWindows};
use wry::WebViewBuilder;

#[derive(Deserialize)]
struct Payload {
    id: String,
    name: String,
    exit_code: Option<i32>,
    lines: String,
    port: u16,
}

fn html_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

fn js_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '<' => out.push_str("\\u003c"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

fn build_html(p: &Payload) -> String {
    let code_text = p
        .exit_code
        .map(|c| c.to_string())
        .unwrap_or_else(|| "?".into());
    let name_html = html_escape(&p.name);
    let lines_html = if p.lines.trim().is_empty() {
        String::new()
    } else {
        format!(
            r#"<pre class="out">{}</pre>"#,
            html_escape(p.lines.trim_end())
        )
    };
    let base = format!("http://127.0.0.1:{}", p.port);
    format!(
        r##"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8" />
<meta name="viewport" content="width=device-width, initial-scale=1" />
<style>
  :root {{
    --bg:#0e1117; --panel:#151a23; --panel2:#1a2130; --border:#26303f;
    --text:#d7dee9; --muted:#7c8899; --accent:#4c8dff; --accent2:#2f6fe0; --red:#ff6b6b;
    --mono: "Cascadia Code","JetBrains Mono",Consolas,"Courier New",monospace;
  }}
  * {{ box-sizing: border-box; }}
  html, body {{ height:100%; margin:0; }}
  body {{
    background: var(--bg); color: var(--text);
    font-family: "Segoe UI", system-ui, -apple-system, sans-serif;
    font-size: 13px; overflow: hidden; user-select: none;
  }}
  .wrap {{ height:100%; display:flex; flex-direction:column; padding:18px; gap:12px; }}
  h2 {{ margin:0; font-size:16px; font-weight:600; }}
  .name {{ display:flex; align-items:center; gap:8px; color: var(--red); min-width:0; }}
  .name strong {{ color: var(--text); font-size:13px; font-weight:600; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }}
  .code {{ margin-left:auto; flex-shrink:0; padding:2px 8px; border-radius:999px; background: rgba(255,107,107,0.12); border:1px solid rgba(255,107,107,0.4); color: var(--red); font-family: var(--mono); font-size:11px; }}
  .out {{ margin:0; padding:10px 12px; max-height:170px; overflow:auto; background:#0b0f16; border:1px solid var(--border); border-radius:8px; color:#ff9b9b; font-family: var(--mono); font-size:11.5px; line-height:1.55; white-space:pre-wrap; word-break:break-word; user-select:text; }}
  .msg {{ min-height:18px; font-size:12px; color: var(--muted); }}
  .msg.err {{ color: var(--red); }}
  .actions {{ margin-top:auto; display:flex; justify-content:flex-end; gap:8px; padding-top:10px; border-top:1px solid var(--border); }}
  .btn {{ padding:6px 12px; border-radius:6px; border:1px solid var(--border); background: var(--panel2); color: var(--text); font-size:12px; cursor:pointer; font-family:inherit; }}
  .btn:hover {{ background:#222c3e; }}
  .btn.primary {{ background: var(--accent2); border-color: var(--accent2); color:#fff; font-weight:600; }}
  .btn.primary:hover {{ background: var(--accent); border-color: var(--accent); }}
  .icon {{ width:16px; height:16px; flex-shrink:0; stroke:currentColor; fill:none; stroke-width:2; }}
</style>
</head>
<body>
  <div class="wrap">
    <h2>Process failed</h2>
    <div class="name">
      <svg class="icon" viewBox="0 0 24 24" aria-hidden="true"><path d="M12 3l7 3v5c0 5-3.5 8.5-7 10-3.5-1.5-7-5-7-10V6l7-3z"/><path d="M9 12l2 2 4-4"/></svg>
      <strong>{name_html}</strong>
      <span class="code">exit code {code_text}</span>
    </div>
    {lines_html}
    <div id="msg" class="msg"></div>
    <div class="actions">
      <button id="close" class="btn" type="button">Close</button>
      <button id="stop" class="btn" type="button">Stop future runs</button>
      <button id="rerun" class="btn primary" type="button">Rerun</button>
    </div>
  </div>
<script>
(function() {{
  const ID = {id_js};
  const BASE = {base_js};
  const msg = document.getElementById('msg');
  const setMsg = (t, bad) => {{ msg.textContent = t || ''; msg.className = 'msg' + (bad ? ' err' : ''); }};
  const closeWin = () => {{
    try {{
      if (window.ipc && typeof window.ipc.postMessage === 'function') {{
        window.ipc.postMessage('close');
      }} else {{
        window.close();
      }}
    }} catch(e) {{}}
  }};
  document.getElementById('close').onclick = closeWin;
  document.getElementById('rerun').onclick = async () => {{
    try {{
      setMsg('Starting...');
      const r = await fetch(BASE + '/api/start', {{
        method:'POST', headers:{{'Content-Type':'application/json'}},
        body: JSON.stringify({{id: ID}})
      }});
      if (!r.ok) {{
        let detail = '';
        try {{ const j = await r.json(); if (j && j.error) detail = String(j.error); }} catch(_) {{}}
        throw new Error(detail || ('Request failed (' + r.status + ')'));
      }}
      setMsg('Started. Closing...');
      setTimeout(closeWin, 250);
    }} catch(e) {{ setMsg(String(e && e.message ? e.message : e), true); }}
  }};
  document.getElementById('stop').onclick = async () => {{
    try {{
      setMsg('Updating...');
      const sr = await fetch(BASE + '/api/state');
      if (!sr.ok) throw new Error('Failed to load state (' + sr.status + ')');
      const s = await sr.json();
      const entry = (s.entries || []).find(e => e.id === ID);
      if (!entry) throw new Error('Entry not found');
      entry.errorPolicy = 'stopRuns';
      entry.halted = true;
      const r = await fetch(BASE + '/api/entries', {{
        method:'PUT', headers:{{'Content-Type':'application/json'}},
        body: JSON.stringify({{entry}})
      }});
      if (!r.ok) {{
        let detail = '';
        try {{ const j = await r.json(); if (j && j.error) detail = String(j.error); }} catch(_) {{}}
        throw new Error(detail || ('Request failed (' + r.status + ')'));
      }}
      setMsg('Future runs stopped. Closing...');
      setTimeout(closeWin, 250);
    }} catch(e) {{ setMsg(String(e && e.message ? e.message : e), true); }}
  }};
  window.addEventListener('keydown', (e) => {{ if (e.key === 'Escape') closeWin(); }});
}})();
</script>
</body>
</html>"##,
        name_html = name_html,
        code_text = code_text,
        lines_html = lines_html,
        id_js = js_string(&p.id),
        base_js = js_string(&base),
    )
}

#[cfg(target_os = "windows")]
fn set_windows_app_id() {
    // Give the popup a distinct AppUserModelID so it never shares taskbar/
    // window-management identity with the main UI.
    use std::ffi::OsStr;
    use std::iter::once;
    use std::os::windows::ffi::OsStrExt;

    #[link(name = "shell32")]
    extern "system" {
        fn SetCurrentProcessExplicitAppUserModelID(app_id: *const u16) -> i32;
    }

    let wide: Vec<u16> = OsStr::new("com.seremtitus.onstartup.errorpopup")
        .encode_wide()
        .chain(once(0))
        .collect();
    unsafe {
        let _ = SetCurrentProcessExplicitAppUserModelID(wide.as_ptr());
    }
}

#[cfg(not(target_os = "windows"))]
fn set_windows_app_id() {}

fn main() -> wry::Result<()> {
    set_windows_app_id();

    let payload_json = std::env::args().nth(1).unwrap_or_else(|| {
        let mut s = String::new();
        let _ = std::io::stdin().read_to_string(&mut s);
        s
    });
    let payload: Payload = match serde_json::from_str(payload_json.trim()) {
        Ok(p) => p,
        Err(_) => std::process::exit(2),
    };

    let event_loop = EventLoop::new();
    let mut builder = WindowBuilder::new()
        .with_title("On Start Up - Process failed")
        .with_inner_size(LogicalSize::new(520.0, 360.0))
        .with_min_inner_size(LogicalSize::new(420.0, 260.0))
        .with_resizable(true)
        .with_always_on_top(true)
        .with_visible(false);
    #[cfg(target_os = "windows")]
    {
        builder = builder.with_skip_taskbar(true);
    }
    let window = builder.build(&event_loop).map_err(|e| {
        wry::Error::Io(std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))
    })?;

    // Keep the popup as a transient alert window: no taskbar entry and avoid
    // interfering with the main UI's taskbar icon/grouping.
    #[cfg(target_os = "windows")]
    {
        let _ = window.set_skip_taskbar(true);
    }

    if let Some(monitor) = window.current_monitor().or_else(|| event_loop.primary_monitor()) {
        let mpos = monitor.position();
        let msize = monitor.size();
        let wsize = window.outer_size();
        let x = mpos.x + (msize.width as i32 - wsize.width as i32) / 2;
        let y = mpos.y + (msize.height as i32 - wsize.height as i32) / 2;
        window.set_outer_position(PhysicalPosition::new(x.max(mpos.x), y.max(mpos.y)));
    }

    let html = build_html(&payload);

    // Use an isolated WebView data directory so popup instances don't share
    // identity/state with the main UI or with each other.
    let data_dir = std::env::temp_dir().join("on-start-up-error-popup-webview");
    let _ = std::fs::create_dir_all(&data_dir);
    let mut web_context = wry::WebContext::new(Some(data_dir));

    #[cfg(any(
        target_os = "windows",
        target_os = "macos",
        target_os = "ios",
        target_os = "android"
    ))]
    let _webview = WebViewBuilder::new_with_web_context(&mut web_context)
        .with_html(html)
        .with_ipc_handler(|req| {
            if req.body() == "close" {
                std::process::exit(0);
            }
        })
        .build(&window)?;

    #[cfg(not(any(
        target_os = "windows",
        target_os = "macos",
        target_os = "ios",
        target_os = "android"
    )))]
    let _webview = {
        use tao::platform::unix::WindowExtUnix;
        use wry::WebViewBuilderExtUnix;
        let vbox = window.default_vbox().unwrap();
        WebViewBuilder::new_with_web_context(&mut web_context)
            .with_html(html)
            .with_ipc_handler(|req| {
                if req.body() == "close" {
                    std::process::exit(0);
                }
            })
            .build_gtk(vbox)?
    };

    window.set_visible(true);
    let _ = window.set_focus();

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;
        if let Event::WindowEvent {
            event: WindowEvent::CloseRequested,
            ..
        } = event
        {
            *control_flow = ControlFlow::Exit;
        }
    });
}