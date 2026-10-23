//! The HTML documents of the web viewer and shares: one inline stylesheet
//! close to the app's look (light and dark through `prefers-color-scheme`),
//! preview-card tags, and the viewer's small Share dialog, the only script.

use crate::render::{escape, Rendered};

const STYLE: &str = r#"
:root{color-scheme:light dark;--bg:#FFFAF1;--desk:#ECE6DB;--text:#282A2F;--muted:#625C55;--faint:#6C645D;--border:#E4DCCF;--card:#F3EFE7;--link:#874016;--accent:#DE945F;--accent-hover:#C97F4A;--on-accent:#282A2F;--sel:rgba(222,148,95,.30);--line:#B8ADA1}
@media (prefers-color-scheme:dark){:root{--bg:#30333A;--desk:#222428;--text:#F3EFE7;--muted:#C6BAAC;--faint:#999DA6;--border:#41444C;--card:#2A2D33;--link:#F0A064;--accent:#F0A064;--accent-hover:#F5B47F;--sel:rgba(240,160,100,.34);--line:#575B64}}
*{box-sizing:border-box}
html{background:var(--desk)}
body{margin:0;background:var(--desk);color:var(--text);font:16px/1.5 "Atkinson Hyperlegible Next","Atkinson Hyperlegible",system-ui,-apple-system,"Segoe UI",sans-serif;-webkit-text-size-adjust:100%}
::selection{background:var(--sel)}
a{color:var(--link)}
.bar{max-width:816px;margin:0 auto;padding:12px 16px;display:flex;align-items:center;gap:8px;font-size:14px}
.brand{display:flex;align-items:center;gap:8px;color:var(--muted);text-decoration:none;font-weight:600;margin-right:auto}
.brand img{width:22px;height:22px;border-radius:6px}
.btn{display:inline-flex;align-items:center;justify-content:center;height:34px;padding:0 14px;border-radius:7px 11px 6px 9px;border:1px solid var(--line);background:var(--bg);color:var(--text);font:600 14px/1 inherit;font-family:inherit;text-decoration:none;cursor:pointer}
.btn:hover{background:var(--card)}
.btn.primary{background:var(--accent);border-color:transparent;color:var(--on-accent);font-weight:700}
.btn.primary:hover{background:var(--accent-hover)}
.btn.small{height:28px;padding:0 10px;font-size:13px}
.sheet{max-width:816px;margin:0 auto 32px;padding:40px 48px 56px;background:var(--bg);border-radius:12px 18px 10px 16px}
.title{font:640 40px/48px "Bricolage Grotesque",system-ui,sans-serif;letter-spacing:-.03em;margin:0 0 16px;overflow-wrap:anywhere}
.title .icon{margin-right:.25em}
.props{border-collapse:collapse;margin:0 0 24px;font-size:14px}
.props th{color:var(--muted);font-weight:400;text-align:left;padding:4px 24px 4px 0;vertical-align:top;white-space:nowrap}
.props td{padding:4px 0}
article{overflow-wrap:anywhere}
article h1,article h2,article h3{font-family:"Bricolage Grotesque",system-ui,sans-serif;font-weight:640;letter-spacing:-.03em;margin:1.4em 0 .4em}
article h1{font-size:30px;line-height:39px}article h2{font-size:24px;line-height:31px}article h3{font-size:20px;line-height:26px}
article p{margin:.5em 0}
article ul,article ol{padding-left:1.5em;margin:.4em 0}
article li::marker{color:var(--accent)}
article li:has(>input[type=checkbox]){list-style:none;margin-left:-1.4em}
article input[type=checkbox]{appearance:none;width:16px;height:16px;margin:0 8px -2px 0;border:1.5px solid var(--faint);border-radius:4px;vertical-align:baseline}
article input[type=checkbox]:checked{background:var(--accent);border-color:var(--accent);box-shadow:inset 0 0 0 2px var(--bg)}
article blockquote{margin:.6em 0;padding:2px 0 2px 14px;border-left:3px solid var(--text)}
article code{font:85% "Fragment Mono",ui-monospace,SFMono-Regular,Menlo,Consolas,monospace;color:var(--link);background:var(--card);padding:1px 4px;border-radius:4px}
article pre{background:var(--card);padding:12px 14px;border-radius:4px;overflow:auto}
article pre code{color:var(--text);background:none;padding:0;font-size:13.6px;line-height:20.4px}
article hr{border:0;border-top:1px solid var(--border);margin:1.2em 0}
article figure{margin:.8em 0}
article img{max-width:100%;height:auto;border-radius:6px;display:block}
article table,.db table{border-collapse:collapse;font-size:14px;margin:.6em 0}
article th,article td,.db th,.db td{border:1px solid var(--border);padding:6px 10px;text-align:left;vertical-align:top}
article th,.db th{font-weight:600;color:var(--muted)}
.db{margin:1em 0}.db-title{font-weight:700;margin-bottom:4px}.db-scroll{overflow-x:auto}
a.pagelink{text-decoration:underline;text-decoration-color:var(--border);text-underline-offset:3px}
span.pagelink{color:inherit}
.subpage{display:block;margin:.3em 0}
.muted{color:var(--muted);font-size:14px}
footer{max-width:816px;margin:0 auto;padding:0 16px 32px;color:var(--faint);font-size:13px;text-align:center}
.center{min-height:100vh;display:flex;align-items:center;justify-content:center;padding:16px}
.card{max-width:420px;width:100%;background:var(--bg);border-radius:12px 18px 10px 16px;padding:32px;text-align:center}
.card img{width:56px;height:56px;border-radius:12px}
.card h1{font:640 24px/1.3 "Bricolage Grotesque",system-ui,sans-serif;letter-spacing:-.03em;margin:12px 0 8px}
.card p{color:var(--muted);margin:0 0 20px}
dialog{position:fixed;inset:0;margin:auto;height:fit-content;max-height:calc(100vh - 32px);overflow:auto;width:min(380px,calc(100vw - 32px));border:1px solid var(--border);border-radius:12px 18px 10px 16px;background:var(--bg);color:var(--text);padding:20px;box-shadow:0 12px 40px rgba(0,0,0,.25)}
[hidden]{display:none!important}
dialog .hint{margin:-4px 0 12px;font-size:13px}
dialog .list-h{font-size:14px;margin:16px 0 6px}
dialog::backdrop{background:rgba(0,0,0,.3)}
dialog h2{font:640 20px/1.3 "Bricolage Grotesque",system-ui,sans-serif;margin:0 0 14px}
.seg{display:flex;border:1px solid var(--line);border-radius:7px 11px 6px 9px;overflow:hidden;margin-bottom:10px}
.seg label{flex:1;text-align:center;padding:7px 0;font-size:14px;cursor:pointer}
.seg input{position:absolute;opacity:0;pointer-events:none}
.seg label:has(input:checked){background:var(--card);font-weight:700}
.share-url{display:flex;gap:6px;margin:12px 0}
.share-url input{flex:1;min-width:0;height:34px;border:1px solid var(--border);border-radius:6px;background:var(--card);color:var(--text);padding:0 8px;font:13px "Fragment Mono",ui-monospace,monospace}
.share-list{list-style:none;padding:0;margin:0;border-top:1px solid var(--border)}
.share-list li{display:flex;align-items:center;gap:6px;padding:8px 0;border-bottom:1px solid var(--border);font-size:14px}
.share-list .what{flex:1}.share-list .when{display:block;color:var(--faint);font-size:12px}
.status{min-height:20px;color:var(--muted);font-size:13px;margin:6px 0 0}
.close{float:right}
@media (max-width:640px){.sheet{padding:24px 16px 40px;border-radius:0}.title{font-size:32px;line-height:40px}.bar{padding:10px 16px}}
"#;

/// The preview card: Open Graph and Twitter tags.
pub(crate) struct Card<'a> {
    pub title: &'a str,
    pub description: &'a str,
    /// Absolute.
    pub image_url: &'a str,
    /// Absolute.
    pub url: &'a str,
}

fn card_tags(c: &Card) -> String {
    let t = escape(c.title);
    let d = escape(c.description);
    let i = escape(c.image_url);
    let u = escape(c.url);
    let mut s = format!(
        "<meta property=\"og:site_name\" content=\"Scratch\">\n\
         <meta property=\"og:type\" content=\"article\">\n\
         <meta property=\"og:title\" content=\"{t}\">\n\
         <meta property=\"og:url\" content=\"{u}\">\n\
         <meta property=\"og:image\" content=\"{i}\">\n\
         <meta name=\"twitter:card\" content=\"summary\">\n\
         <meta name=\"twitter:title\" content=\"{t}\">\n\
         <meta name=\"twitter:image\" content=\"{i}\">\n"
    );
    if !c.description.is_empty() {
        s.push_str(&format!(
            "<meta name=\"description\" content=\"{d}\">\n\
             <meta property=\"og:description\" content=\"{d}\">\n\
             <meta name=\"twitter:description\" content=\"{d}\">\n"
        ));
    }
    s
}

fn head(title: &str, card: &Card) -> String {
    format!(
        "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
         <meta name=\"robots\" content=\"noindex\">\n\
         <meta name=\"referrer\" content=\"no-referrer\">\n\
         <title>{}</title>\n<link rel=\"icon\" href=\"/icon.png\">\n{}<style>{STYLE}</style>\n</head>\n",
        escape(title),
        card_tags(card)
    )
}

/// What surrounds the page.
pub(crate) enum Chrome<'a> {
    /// A share: no controls, no script.
    Share,
    /// The signed-in viewer: Open in app, Share (dialog) and Sign out.
    Viewer { page_id: &'a str, nonce: &'a str },
}

fn page_body(r: &Rendered) -> String {
    let icon = r
        .icon
        .as_deref()
        .map(|i| format!("<span class=\"icon\">{}</span>", escape(i)))
        .unwrap_or_default();
    format!(
        "<main class=\"sheet\">\n<h1 class=\"title\">{icon}{}</h1>\n{}\n<article>\n{}</article>\n</main>\n",
        escape(&r.title),
        r.props_html,
        r.body_html
    )
}

const BRAND: &str = "<span class=\"brand\"><img src=\"/icon.png\" alt=\"\">Scratch</span>";

/// A rendered page as a whole document.
pub(crate) fn page_document(r: &Rendered, card: &Card, chrome: Chrome) -> String {
    let title = format!("{} · Scratch", r.title);
    let mut html = head(&title, card);
    match chrome {
        Chrome::Share => {
            html.push_str("<body>\n<header class=\"bar\"><span class=\"brand\"><img src=\"/icon.png\" alt=\"\">Scratch</span></header>\n");
            html.push_str(&page_body(r));
            html.push_str("<footer>Shared from Scratch</footer>\n</body>\n</html>\n");
        }
        Chrome::Viewer { page_id, nonce } => {
            let id = escape(page_id);
            html.push_str(&format!(
                "<body data-page=\"{id}\">\n<header class=\"bar\">{BRAND}\
                 <a class=\"btn\" href=\"notes://page/{id}\">Open in app</a>\
                 <button class=\"btn primary\" id=\"share-open\" type=\"button\">Share</button>\
                 <form method=\"post\" action=\"/auth/web/logout\" style=\"margin:0\">\
                 <input type=\"hidden\" name=\"return\" value=\"/open/{id}\">\
                 <button class=\"btn\" type=\"submit\">Sign out</button></form></header>\n"
            ));
            html.push_str(&page_body(r));
            html.push_str(SHARE_DIALOG);
            html.push_str(&format!(
                "<script nonce=\"{}\">{SHARE_SCRIPT}</script>\n",
                escape(nonce)
            ));
            html.push_str("</body>\n</html>\n");
        }
    }
    html
}

const SHARE_DIALOG: &str = r#"<dialog id="share" aria-labelledby="share-h">
<form method="dialog"><button class="btn small close" aria-label="Close">Close</button></form>
<h2 id="share-h">Share this note</h2>
<form id="share-form">
<div class="seg" role="radiogroup" aria-label="Who can open it">
<label><input type="radio" name="visibility" value="public">Public</label>
<label><input type="radio" name="visibility" value="private" checked>Private</label>
</div>
<p class="muted hint">Public: anyone with the link. Private: only people signed in to this hub.</p>
<div class="seg" role="radiogroup" aria-label="What the link shows">
<label><input type="radio" name="kind" value="snapshot" checked>This version</label>
<label><input type="radio" name="kind" value="live">Live link</label>
</div>
<p class="muted hint">This version: a copy of the page as it is now. Live link: always the latest.</p>
<button class="btn primary" type="submit" style="width:100%">Create link</button>
</form>
<div class="share-url" id="share-made" hidden><input id="share-url" readonly aria-label="Link"><button class="btn small" id="share-copy" type="button">Copy</button></div>
<p class="status" id="share-status" role="status"></p>
<h3 class="list-h">Links to this page</h3>
<ul class="share-list" id="share-list"></ul>
</dialog>
"#;

const SHARE_SCRIPT: &str = r#"
(function(){
const page=document.body.dataset.page,dlg=document.getElementById('share'),list=document.getElementById('share-list'),status=document.getElementById('share-status');
function say(t){status.textContent=t}
async function api(method,path,body){const o={method,credentials:'same-origin',headers:{}};if(body){o.headers['Content-Type']='application/json';o.body=JSON.stringify(body)}const r=await fetch(path,o);if(!r.ok){let e=r.status;try{e=(await r.json()).error||e}catch(_){}throw new Error(e)}return r.status===204?null:r.json()}
function what(s){return (s.visibility==='public'?'Public':'Private')+' · '+(s.kind==='live'?'Live link':'This version')}
async function copy(url){try{await navigator.clipboard.writeText(url);say('Link copied')}catch(_){say('Copy the link above')}}
async function load(){list.textContent='';try{const r=await api('GET','/shares?page='+encodeURIComponent(page));for(const s of r.shares){const li=document.createElement('li'),w=document.createElement('span'),when=document.createElement('span');w.className='what';w.textContent=what(s);when.className='when';when.textContent=new Date(s.created).toLocaleString();w.appendChild(when);const c=document.createElement('button');c.className='btn small';c.type='button';c.textContent='Copy';c.onclick=()=>copy(s.url);const x=document.createElement('button');x.className='btn small';x.type='button';x.textContent='Revoke';x.onclick=async()=>{try{await api('DELETE','/shares/'+encodeURIComponent(s.id));say('Link revoked');load()}catch(e){say('Could not revoke: '+e.message)}};li.append(w,c,x);list.appendChild(li)}}catch(e){say('Could not load links: '+e.message)}}
document.getElementById('share-open').onclick=()=>{say('');dlg.showModal();load()};
document.getElementById('share-form').onsubmit=async(ev)=>{ev.preventDefault();const f=new FormData(ev.target);try{const s=await api('POST','/shares',{page,visibility:f.get('visibility'),kind:f.get('kind')});document.getElementById('share-made').hidden=false;const u=document.getElementById('share-url');u.value=s.url;u.select();await copy(s.url);load()}catch(e){say('Could not create the link: '+e.message)}};
document.getElementById('share-copy').onclick=()=>copy(document.getElementById('share-url').value);
})();
"#;

/// A centred message card (errors, revoked links, private shares).
pub(crate) fn message_document(
    title: &str,
    text: &str,
    action: Option<(&str, &str)>,
    card: Option<&Card>,
) -> String {
    let empty = Card {
        title,
        description: "",
        image_url: "/icon.png",
        url: "",
    };
    let mut html = head(&format!("{title} · Scratch"), card.unwrap_or(&empty));
    let button = action
        .map(|(href, label)| {
            format!(
                "<a class=\"btn primary\" href=\"{}\">{}</a>",
                escape(href),
                escape(label)
            )
        })
        .unwrap_or_default();
    html.push_str(&format!(
        "<body>\n<div class=\"center\"><main class=\"card\"><img src=\"/icon.png\" alt=\"\">\
         <h1>{}</h1><p>{}</p>{button}</main></div>\n</body>\n</html>\n",
        escape(title),
        escape(text)
    ));
    html
}
