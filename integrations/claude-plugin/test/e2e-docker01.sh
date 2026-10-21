#!/usr/bin/env bash
# Run from Git Bash on the PC. No local builds or permanent Claude installation.
# Leaves the owned staging hub/volume and the private token file for reuse.
set -Eeuo pipefail
set +x
umask 077

usage() {
  cat <<'EOF'
Usage: bash integrations/claude-plugin/test/e2e-docker01.sh [--reuse-hub] [--skip-import]
Builds HEAD plus BubblePaw 32986c5 on docker01, imports Claude memories, and
tests a temporary plugin using write/read agent tokens. Existing Claude settings,
plugins and memory files are hashed before and after. Evidence is token-redacted.
Environment overrides: E2E_SCRATCH, E2E_BUBBLEPAW, E2E_PORT (new hub only),
E2E_CLAUDE_MODEL, E2E_CLAUDE_TIMEOUT (seconds, default 240).
EOF
}
reuse=0 skip_import=0
for arg in "$@"; do
  case "$arg" in
    --reuse-hub) reuse=1 ;; --skip-import) skip_import=1 ;;
    -h|--help) usage; exit 0 ;; *) usage >&2; exit 2 ;;
  esac
done
die() { printf 'E2E stopped: %s\n' "$*" >&2; exit 1; }
say() { printf 'E2E: %s\n' "$*"; }
root=$(git rev-parse --show-toplevel)
cd "$root"
# Any branch works: the build archives the current HEAD commit, recorded in notes-head.txt.
# Share the archive and reuse comparison inputs so test/wiki-only commits can
# reuse a hub while changes to any shipped build input require a new image.
build_inputs=(Cargo.toml Cargo.lock .cargo .dockerignore .gitattributes crates app/rust prototypes/tauri/src-tauri deploy/hub)
scratch=${E2E_SCRATCH:-/c/Users/Admin/AppData/Local/Temp/claude/C--Users-Admin-Documents-Projects-notes-app/4ccd6103-feb1-4413-b3d0-c63aa98960e4/scratchpad/lanes/agent-access}
scratch=$(cygpath -u "$scratch")
mkdir -p "$scratch"
scratch=$(cd "$scratch" && pwd -P)
token_file="$scratch/e2e-tokens.env"
run_id="$(date -u +%Y%m%dT%H%M%SZ)-$$"
run="$scratch/e2e-$run_id"
mkdir -p "$run"
ssh=/c/Windows/System32/OpenSSH/ssh.exe
notes=/f/build/notes-cli/notes.exe
projects="$(cygpath -u "$USERPROFILE")/.claude/projects"
bubblepaw=$(cygpath -u "${E2E_BUBBLEPAW:-$USERPROFILE/Documents/Projects/BubblePaw-base-agent}")
container='notes-hub-e2e'
volume='notes-hub-e2e-data'
image='notes-hub:e2e'
remote_build="/home/dev/.cache/notes-hub-e2e/$run_id"
capture_pid='' plugin='' snapshot_ready=0 all_checks_passed=0 remote_build_created=0
for program in git tar gzip curl python claude timeout rg sed; do command -v "$program" >/dev/null || die "missing $program"; done
[[ -x "$ssh" && -f "$notes" ]] || die 'Windows OpenSSH or F:/build/notes-cli/notes.exe missing'
export PATH="/f/build/notes-cli:$PATH"
export CLAUDE_CODE_DISABLE_AUTO_MEMORY=1 DISABLE_AUTOUPDATER=1
unset CLAUDE_PLUGIN_OPTION_HUB_URL CLAUDE_PLUGIN_OPTION_HUB_TOKEN
remote() { MSYS_NO_PATHCONV=1 "$ssh" -o BatchMode=yes docker01 "$@"; }
# docker01 address for published ports: E2E_DOCKER_IP, else the HostName of the ssh alias.
docker_ip=${E2E_DOCKER_IP:-$("$ssh" -G docker01 | awk '$1=="hostname"{print $2; exit}')}
docker_ip=${docker_ip//$'\r'/}
[[ "$docker_ip" =~ ^[0-9]+\.[0-9]+\.[0-9]+\.[0-9]+$ ]] || die "docker01 address is not an IPv4 address; set E2E_DOCKER_IP"
export E2E_DOCKER_IP="$docker_ip"
# SSH is explicit: Docker's Git Bash SSH transport may pick the wrong client.
dkr() {
  local quoted='' word
  for word in "$@"; do printf -v word '%q' "$word"; quoted+=" $word"; done
  remote "docker$quoted"
}

# This helper handles private values in memory. Raw Claude streams and packet
# captures never reach a file. Evidence files contain only redacted text.
cat > "$run/helper.py" <<'PY'
import os, sys, json, hashlib, pathlib, re, shutil, struct, socket, subprocess

def secrets():
    p = pathlib.Path(os.environ['E2E_TOKEN_FILE'])
    if not p.exists(): return []
    return [line.split('=',1)[1].strip() for line in p.read_text().splitlines()
            if line.startswith(('E2E_WRITE_TOKEN=', 'E2E_READ_TOKEN='))]

def scrub(s):
    for token in secrets():
        if token: s = s.replace(token, '[redacted]')
    return re.sub(r'(?i)(Bearer\s+)[^\s"\\]+', r'\1[redacted]', s)

mode = sys.argv[1]
if mode == 'private-file':
    p=pathlib.Path(os.environ['E2E_TOKEN_FILE'])
    if not p.exists(): p.touch(mode=0o600,exist_ok=False)
    # NTFS does not represent chmod 0600 accurately. Verify an existing private
    # DACL first; change only access rules when needed, before writing secrets.
    ps=r'''
$ErrorActionPreference='Stop'
[Console]::OutputEncoding=[System.Text.UTF8Encoding]::new($false)
$tokenPath=$env:E2E_TOKEN_FILE
$userSid=[System.Security.Principal.WindowsIdentity]::GetCurrent().User
$sections=[System.Security.AccessControl.AccessControlSections]::Access -bor [System.Security.AccessControl.AccessControlSections]::Owner -bor [System.Security.AccessControl.AccessControlSections]::Group
$fileInfo=[System.IO.FileInfo]::new($tokenPath)
function TestOwnerOnlyAcl($acl) {
    $rules=@($acl.GetAccessRules($true,$true,[System.Security.Principal.SecurityIdentifier]))
    return ($acl.AreAccessRulesProtected -and $rules.Count -eq 1 -and $rules[0].IdentityReference.Value -eq $userSid.Value -and $rules[0].AccessControlType -eq 'Allow' -and -not $rules[0].IsInherited -and $rules[0].FileSystemRights -eq [System.Security.AccessControl.FileSystemRights]::FullControl)
}
$fileAcl=$fileInfo.GetAccessControl($sections)
$alreadyValid=TestOwnerOnlyAcl $fileAcl
if (-not $alreadyValid) {
    # Preserve owner/group and never read or write the SACL. Persisting only
    # changed DACL sections needs no SeSecurityPrivilege in unelevated PS 5.
    $fileAcl.SetAccessRuleProtection($true,$false)
    foreach ($entry in @($fileAcl.GetAccessRules($true,$true,[System.Security.Principal.SecurityIdentifier]))) {
        $fileAcl.RemoveAccessRuleSpecific($entry)
    }
    $rule=[System.Security.AccessControl.FileSystemAccessRule]::new($userSid,'FullControl','Allow')
    $fileAcl.AddAccessRule($rule)
    $fileInfo.SetAccessControl($fileAcl)
}
$actual=$fileInfo.GetAccessControl($sections)
$entries=@($actual.GetAccessRules($true,$true,[System.Security.Principal.SecurityIdentifier]))
if (-not (TestOwnerOnlyAcl $actual)) { throw 'Token file is not owner-only' }
@{protected=$actual.AreAccessRulesProtected;owner_sid=$actual.GetOwner([System.Security.Principal.SecurityIdentifier]).Value;allowed_user_sid=$userSid.Value;access_rule_count=$entries.Count;permission='owner-only FullControl';acl_changed=(-not $alreadyValid)} | ConvertTo-Json -Compress
'''
    result=subprocess.run(['powershell.exe','-NoProfile','-NonInteractive','-Command',ps],capture_output=True)
    if result.returncode: raise RuntimeError('cannot establish owner-only token file ACL')
    # Windows PowerShell may emit localized diagnostics using a legacy code
    # page; binary capture avoids decoding failures masking the ACL result.
    report=json.loads(result.stdout.decode('utf-8-sig',errors='replace').strip())
    print(scrub(json.dumps(report)))
elif mode == 'sanitize':
    for line in sys.stdin: sys.stdout.write(scrub(line)); sys.stdout.flush()
elif mode == 'feedback-tools':
    import urllib.request
    lists = {}
    for scope in ('READ', 'WRITE'):
        request = urllib.request.Request(os.environ['NOTES_HUB_URL'] + '/mcp',
            data=json.dumps({'jsonrpc':'2.0','id':1,'method':'tools/list'}).encode(),
            headers={'Content-Type':'application/json',
                     'Authorization':'Bearer ' + os.environ['E2E_' + scope + '_TOKEN']})
        with urllib.request.urlopen(request, timeout=15) as response:
            result = json.load(response)
        names = {tool['name'] for tool in result['result']['tools']}
        assert 'feedback_list' not in names, scope + ' token still lists feedback_list'
        assert ('feedback_submit' in names) == (scope == 'WRITE'), 'feedback_submit scope mismatch'
        lists[scope.lower()] = sorted(name for name in names if name.startswith('feedback_'))
    print(json.dumps(lists, indent=2))
elif mode == 'snapshot':
    home = pathlib.Path(sys.argv[3]) if len(sys.argv)>3 else pathlib.Path.home()/'.claude'
    paths = [home/'settings.json', home/'settings.local.json', home/'CLAUDE.md']
    paths += sorted(p for p in home.glob('settings*.json') if p not in paths)
    paths += [home/'plugins'] + sorted((home/'projects').glob('*/memory'))
    state = {}
    # plugins/synced/ is Claude Code's cloud plugin-sync cache. Any running Claude
    # Code session (other sessions run on this PC) rewrites it, so this test does
    # not change it and it is excluded as a whole. Installed Claude 2.1.293 also
    # writes plugin-cache .in_use/<String(process.pid)> leases and deletes them
    # on exit. Exclude only fully decimal ASCII filenames with .in_use as the
    # direct parent beneath plugins/cache; nonnumeric files remain protected.
    # Installed plugins and their code outside synced/, marketplace configuration
    # outside synced/, all settings*.json, CLAUDE.md and project memory stay protected.
    exclusion_patterns = ['plugins/synced/**',
                          'plugins/cache/**/.in_use/[0-9]*']
    excluded_paths = []
    for target in paths:
        if not target.exists(): state[str(target)] = {'missing':True}; continue
        items = [target] if target.is_file() else sorted(target.rglob('*'))
        for p in items:
            if not p.is_file(): continue
            relative = p.relative_to(home)
            sync_cache = relative.parts[:2]==('plugins','synced')
            pid_lease = (relative.parts[:2]==('plugins','cache') and
                         relative.parent.name=='.in_use' and re.fullmatch(r'[0-9]+',relative.name))
            if sync_cache or pid_lease:
                excluded_paths.append(relative.as_posix())
                continue
            stat = p.stat()
            state[str(p)] = {'sha256':hashlib.sha256(p.read_bytes()).hexdigest(),
                             'mtime_ns':stat.st_mtime_ns, 'size':stat.st_size}
    output = pathlib.Path(sys.argv[2])
    output.write_text(json.dumps(state,sort_keys=True,indent=2))
    output.with_name(output.stem+'-exclusions.json').write_text(json.dumps({
        'excluded_patterns':exclusion_patterns,'excluded_paths':sorted(excluded_paths)
    },sort_keys=True,indent=2))
elif mode == 'compare':
    a,b = (json.loads(pathlib.Path(p).read_text()) for p in sys.argv[2:4])
    changed = sorted(p for p in set(a)|set(b) if a.get(p) != b.get(p))
    print(json.dumps({'protected_files':len(a),'changed_paths':changed},indent=2))
    sys.exit(bool(changed))
elif mode == 'plugin':
    dest, style, scope = pathlib.Path(sys.argv[2]), sys.argv[3], sys.argv[4]
    token = os.environ['E2E_WRITE_TOKEN' if scope == 'write' else 'E2E_READ_TOKEN']
    url = os.environ['NOTES_HUB_URL']
    manifest = dest/'.claude-plugin/plugin.json'
    obj = json.loads(manifest.read_text())
    obj['userConfig']['hub_url']['default'] = url
    obj['userConfig']['hub_token']['default'] = token
    obj['userConfig']['hub_token']['required'] = False
    manifest.write_text(json.dumps(obj,indent=2)+'\n')
    if style == 'literal':
        (dest/'.mcp.json').write_text(json.dumps({'mcpServers':{'notes':{
            'type':'http','url':url+'/mcp','headers':{'Authorization':'Bearer '+token}}}},indent=2)+'\n')
    else:
        # The shipped .mcp.json signs in with OAuth; the headless test uses the token.
        (dest/'.mcp.json').write_text(json.dumps({'mcpServers':{'notes':{
            'type':'http','url':'${user_config.hub_url}/mcp',
            'headers':{'Authorization':'Bearer ${user_config.hub_token}'}}}},indent=2)+'\n')
    # Wrap only the disposable hook to record its actual stdout, also passed
    # unchanged into Claude's SessionStart context. No settings install needed.
    original = dest/'scripts/session-start.sh'
    if not (dest/'scripts/session-start-original.sh').exists():
        original.rename(dest/'scripts/session-start-original.sh')
    original.write_text('#!/usr/bin/env bash\nset -o pipefail\n'
        '"${CLAUDE_PLUGIN_ROOT}/scripts/session-start-original.sh" | '
        'tee "$E2E_HOOK_LOG"\n')
elif mode == 'import-check':
    rows = re.findall(r'^([^\n:]+): new=(\d+) updated=(\d+) unchanged=(\d+) conflicts=(\d+)$',
                      pathlib.Path(sys.argv[2]).read_text(),re.M)
    assert rows, 'no import counts found'
    assert all(int(n)==0 and int(u)==0 for _,n,u,_,_ in rows), 'second import changed memories'
    assert sum(int(c) for _,_,_,c,_ in rows)>0, 'no unchanged memories verified'
    print(json.dumps({'projects':len(rows),'unchanged':sum(int(c) for _,_,_,c,_ in rows)}))
elif mode == 'import-counts':
    reports=[]
    for path in sys.argv[2:]:
        rows=re.findall(r'^([^\n:]+): new=(\d+) updated=(\d+) unchanged=(\d+) conflicts=(\d+)$',
                        pathlib.Path(path).read_text(),re.M)
        assert rows, 'missing import counts'
        reports.append({p:sum(map(int,(n,u,c))) for p,n,u,c,_ in rows})
    assert sum(reports[0].values())==int(os.environ.get('E2E_EXPECTED_IMPORT_COUNT','90')), 'source memory count changed'
    assert all(r==reports[0] for r in reports), 'import count differs from offline source inventory'
    print(json.dumps({'source_memories':sum(reports[0].values()),'project_counts':reports[0],
                      'reports_verified':len(reports)},indent=2))
elif mode == 'stream-check':
    events=[]
    for line in pathlib.Path(sys.argv[2]).read_text().splitlines():
        try: events.append(json.loads(line))
        except ValueError: pass
    calls=[]; results={}; init={}
    for event in events:
        if event.get('type')=='system' and event.get('subtype')=='init': init=event
        msg=event.get('message',{})
        for c in msg.get('content',[]) if isinstance(msg,dict) else []:
            if c.get('type')=='tool_use': calls.append(c)
            if c.get('type')=='tool_result': results[c.get('tool_use_id')]=c
    servers=init.get('mcp_servers',[])
    connected=any('notes' in s.get('name','') and s.get('status')=='connected' for s in servers)
    def ok(name):
        return any(c.get('name')=='mcp__plugin_notes_notes__'+name and
                   c.get('id') in results and not results[c['id']].get('is_error',False) for c in calls)
    hook_events=[e for e in events if e.get('type')=='system' and
                 'hook' in e.get('subtype','') and e.get('hook_event')=='SessionStart']
    if hook_events:
        assert any('global/tools/e2e-seed' in json.dumps(e) for e in hook_events), 'SessionStart stream event missing seeded global index'
    report={'connected':connected,'servers':servers,'tool_calls':[c.get('name') for c in calls],
            'advertised_plugin_tools':[n for n in init.get('tools',[]) if n.startswith('mcp__plugin_notes_notes__')],
            'session_start_stream_events':len(hook_events),
            'successful_tools':[n for n in ['search_notes','memory_read','memory_write'] if ok(n)]}
    print(scrub(json.dumps(report,indent=2)))
    kind=sys.argv[3]
    if kind=='probe': sys.exit(0 if connected and ok('search_notes') and ok('memory_read') else 1)
    assert connected, 'notes MCP not connected'
    assert ok('search_notes') and ok('memory_read'), 'missing successful search/read calls'
    if kind=='write':
        assert ok('memory_write'), 'missing successful memory_write'
        wanted=os.environ['E2E_MARKER']
        assert any(c.get('name','').endswith('__memory_write') and
                   c.get('input',{}).get('path')=='global/tools/e2e-check' and
                   wanted in c.get('input',{}).get('body','') for c in calls), 'wrong marker/path'
    if kind=='read':
        writes=[c for c in calls if c.get('name','').endswith(('__memory_write','__memory_update','__memory_delete'))]
        assert all(results.get(c.get('id'),{}).get('is_error',False) for c in writes), 'read session wrote memory'
        advertised=[n for n in init.get('tools',[]) if n.startswith('mcp__plugin_notes_notes__') and
                    n.endswith(('__memory_write','__memory_update','__memory_delete'))]
        assert not advertised or writes, 'write tools advertised but refusal was not exercised'
        assert not any(c.get('name') in ['Bash','Write','Edit'] for c in calls), 'read session used disallowed tool'
elif mode == 'packets':
    # Classic pcap -> IPv4 TCP reassembly -> HTTP header *names* only. Headers
    # and packet bytes stay in memory and are discarded after each request.
    src=sys.stdin.buffer
    def exact(n):
        out=bytearray()
        while len(out)<n:
            part=src.read(n-len(out))
            if not part: return None
            out.extend(part)
        return bytes(out)
    header=exact(24)
    assert header, 'empty packet capture'
    endian='<' if header[:4] in (b'\xd4\xc3\xb2\xa1',b'\x4d\x3c\xb2\xa1') else '>'
    link=struct.unpack(endian+'I',header[20:24])[0]
    assert link==1, 'capture is not Ethernet'
    flows={}; total=0; origins=0; bearer=0; packets=0
    while True:
        rec=exact(16)
        if rec is None: break
        caplen=struct.unpack(endian+'IIII',rec)[2]
        assert caplen<=1<<20, 'oversized packet'
        packet=exact(caplen)
        if packet is None: break
        packets+=1
        if len(packet)<54 or packet[12:14]!=b'\x08\x00': continue
        ip=packet[14:]; ihl=(ip[0]&15)*4
        if ip[9]!=6 or len(ip)<ihl+20: continue
        tcp=ip[ihl:]; sport,dport,seq=struct.unpack('!HHI',tcp[:8])
        if dport!=8080: continue
        payload=tcp[(tcp[12]>>4)*4:struct.unpack('!H',ip[2:4])[0]-ihl]
        if not payload: continue
        key=(ip[12:16],ip[16:20],sport,dport)
        f=flows.setdefault(key,{'next':seq,'data':bytearray(),'pending':{},'skip':0})
        # Retransmits overlap; out-of-order segments are drained in order.
        if seq+len(payload)<=f['next']: continue
        if seq<f['next']: payload=payload[f['next']-seq:]; seq=f['next']
        f['pending'][seq]=payload
        while f['next'] in f['pending']:
            chunk=f['pending'].pop(f['next']); f['next']+=len(chunk); f['data'].extend(chunk)
        assert sum(len(x['data'])+sum(map(len,x['pending'].values())) for x in flows.values())<64<<20, 'capture buffer limit'
        data=f['data']
        while True:
            if f['skip']:
                n=min(f['skip'],len(data)); del data[:n]; f['skip']-=n
                if f['skip']: break
            end=data.find(b'\r\n\r\n')
            if end<0: break
            lines=bytes(data[:end]).split(b'\r\n'); del data[:end+4]
            fields={}
            for line in lines[1:]:
                if b':' in line:
                    name,value=line.split(b':',1); fields[name.strip().lower()]=value.strip()
            f['skip']=int(fields.get(b'content-length',b'0'))
            request=lines[0].split()
            if len(request)>=2 and request[1]==b'/mcp':
                total+=1; origins+=b'origin' in fields
                bearer+=fields.get(b'authorization',b'').startswith(b'Bearer ')
                print(json.dumps({'request':total,'method':request[0].decode('ascii','replace'),
                    'path':'/mcp','origin_present':b'origin' in fields,
                    'bearer_present':fields.get(b'authorization',b'').startswith(b'Bearer '),
                    'header_names':[k.decode('ascii','replace') for k in fields]}),flush=True)
    print(json.dumps({'summary':True,'packets':packets,'mcp_requests':total,
                      'origin_requests':origins,'bearer_requests':bearer}),flush=True)
    assert total>0 and bearer==total and origins==0, 'no proof of bearer/no-Origin MCP requests'
PY
E2E_TOKEN_FILE=$(cygpath -m "$token_file")
export E2E_TOKEN_FILE
py() { python "$run/helper.py" "$@"; }
sanitize() { py sanitize; }
stop_capture() {
  if [[ -n "$capture_pid" ]]; then
    local owned_pid=$capture_pid
    capture_pid=''
    # shellcheck disable=SC2016 # The owned container's sh expands its own PID variables.
    dkr exec -u 0 "$container" sh -c 'if test -f /tmp/notes-e2e-capture.pid; then pid=$(cat /tmp/notes-e2e-capture.pid); case "$(tr "\000" " " < /proc/$pid/cmdline 2>/dev/null)" in *timeout*1800*tcpdump*) kill -INT "$pid" 2>/dev/null || true;; esac; rm -f /tmp/notes-e2e-capture.pid; fi' >/dev/null 2>&1 || true
    wait "$owned_pid" || return 1
  fi
}
cleanup() {
  local status=$?
  trap - EXIT INT TERM
  stop_capture || status=1
  if [[ -n "$plugin" && "$plugin" == "$run/"* ]]; then rm -rf -- "$plugin"; fi
  if (( remote_build_created )) && [[ "$remote_build" =~ ^/home/dev/\.cache/notes-hub-e2e/[0-9]{8}T[0-9]{6}Z-[0-9]+$ ]]; then
    remote "rm -rf -- '$remote_build'" >/dev/null 2>&1 || status=1
  fi
  unset E2E_WRITE_TOKEN E2E_READ_TOKEN NOTES_TOKEN CLAUDE_PLUGIN_OPTION_HUB_TOKEN
  if (( snapshot_ready )); then
    if py snapshot "$(cygpath -m "$run/protected-after.json")" &&
       py compare "$(cygpath -m "$run/protected-before.json")" "$(cygpath -m "$run/protected-after.json")" > "$run/protected-comparison.json"; then
      say 'Protected Claude settings, plugins and source memory hashes/mtimes unchanged'
    else
      printf 'E2E failed: protected Claude state check did not pass (see %s)\n' "$(cygpath -m "$run/protected-comparison.json")" >&2
      status=1
    fi
  fi
  # Exit 0 only when the main body ran every check to its final line.
  if (( status == 0 && ! all_checks_passed )); then
    printf 'E2E failed: script ended before all checks completed\n' >&2
    status=1
  fi
  if (( status == 0 )); then say 'All E2E checks passed'; fi
  printf 'E2E evidence: %s\n' "$(cygpath -m "$run")"
  exit "$status"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
py snapshot "$(cygpath -m "$run/protected-before.json")"
snapshot_ready=1
git rev-parse HEAD > "$run/notes-head.txt"
git -C "$bubblepaw" rev-parse '32986c5^{commit}' > "$run/bubblepaw-head.txt"
source_head=$(cat "$run/notes-head.txt")
bubble_head=$(cat "$run/bubblepaw-head.txt")

if (( ! reuse )); then
  say 'Archiving immutable sources and building notes-hub:e2e on docker01'
  mkdir -p "$run/build/notes-app" "$run/build/bubblepaw"
  git archive HEAD "${build_inputs[@]}" | tar -xf - -C "$run/build/notes-app"
  git -C "$bubblepaw" archive 32986c5 Cargo.toml crates/bubblepaw-core crates/bubblepaw-runner extensions/claude-code | tar -xf - -C "$run/build/bubblepaw"
  mkdir -p "$run/build/notes-app/e2e-inspector/src"
  cat > "$run/build/notes-app/e2e-inspector/Cargo.toml" <<'EOF'
[package]
name = "notes-e2e-inspector"
version = "0.0.0"
edition = "2021"
[workspace]
[dependencies]
vault_core = { path = "/src/notes-app/crates/core" }
serde_json = "1"
[profile.release]
debug = false
strip = true
lto = "thin"
codegen-units = 4
EOF
  cat > "$run/build/notes-app/e2e-inspector/src/main.rs" <<'EOF'
use vault_core::Vault;
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = std::env::args().nth(1).ok_or("missing snapshot directory")?;
    let vault = Vault::open(root)?;
    let pages = vault.list_pages(None, 0, 10000)?;
    let globals: Vec<_> = pages.iter().filter(|p| p.title == "global").collect();
    let tools: Vec<_> = pages.iter().filter(|p| p.title == "tools" &&
        globals.iter().any(|g| p.parent.as_ref() == Some(&g.id))).collect();
    let matches: Vec<_> = pages.iter().filter(|p| p.title == "e2e-check" &&
        tools.iter().any(|t| p.parent.as_ref() == Some(&t.id))).collect();
    if matches.len() != 1 { return Err("expected exactly one global/tools/e2e-check".into()); }
    let versions = vault.list_versions(matches[0].id.clone())?;
    if versions.is_empty() || !versions.iter().all(|v| v.device == "agent:e2e-claude") {
        return Err("history actor mismatch".into());
    }
    let rows: Vec<_> = versions.iter().map(|v| serde_json::json!({
        "actor":v.device,"time_ms":v.time_ms,"summary":v.summary,"changes":v.changes
    })).collect();
    println!("{}", serde_json::to_string_pretty(&serde_json::json!({
        "path":"global/tools/e2e-check","verified":true,"versions":rows
    }))?);
    Ok(())
}
EOF
  sed '/^WORKDIR \/src\/notes-app$/a ENV CARGO_BUILD_JOBS=2' "$run/build/notes-app/deploy/hub/Dockerfile" > "$run/build/notes-app/deploy/hub/Dockerfile.e2e"
  cat >> "$run/build/notes-app/deploy/hub/Dockerfile.e2e" <<'EOF'

# Test-only inspector; the source Dockerfile and workspace remain unchanged.
FROM builder AS e2e-inspector-build
COPY e2e-inspector /tmp/e2e-inspector
RUN --mount=type=cache,id=notes-hub-registry,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,id=notes-hub-git,target=/usr/local/cargo/git,sharing=locked \
    --mount=type=cache,id=notes-hub-target-amd64,target=/src/notes-app/target,sharing=locked \
    CARGO_TARGET_DIR=/src/notes-app/target cargo build --release --manifest-path /tmp/e2e-inspector/Cargo.toml \
    && install -Dm755 /src/notes-app/target/release/notes-e2e-inspector /out/notes-e2e-inspector
FROM runtime AS e2e-runtime
COPY --from=e2e-inspector-build /out/notes-e2e-inspector /usr/local/bin/notes-e2e-inspector
EOF
  remote_build_created=1
  tar -czf - -C "$run/build" notes-app bubblepaw |
    remote "mkdir -p '$remote_build' && tar -xzf - -C '$remote_build'"
  remote "cd '$remote_build' && docker build --label notes.e2e.source='$source_head' --label notes.e2e.bubblepaw='$bubble_head' --build-context bubblepaw=./bubblepaw -f notes-app/deploy/hub/Dockerfile.e2e -t notes-hub:e2e ./notes-app" 2>&1 | sanitize > "$run/build.log"
fi
# A full run replaces the owned staging container so it runs the image just
# built. Only a container carrying this harness's label is removed; the data
# volume and the private token file are kept.
if (( ! reuse )) && dkr container inspect "$container" > /dev/null 2>&1; then
  owner=$(dkr container inspect -f '{{index .Config.Labels "notes.e2e"}}' "$container")
  owner=${owner//$'\r'/}
  [[ "$owner" == claude-plugin ]] || die "container $container is not owned by this harness; not replacing it"
  say "Replacing owned staging container $container (volume $volume kept)"
  dkr rm -f "$container" > /dev/null
fi
if ! dkr container inspect "$container" > "$run/container.json" 2>/dev/null; then
  (( ! reuse )) || die '--reuse-hub requested but notes-hub-e2e does not exist'
  port=${E2E_PORT:-}
  if [[ -z "$port" ]]; then
    port=$(remote "python3 - <<'PY'
import socket
for candidate in range(18081,18101):
    with socket.socket() as listener:
        try:
            listener.bind(('$docker_ip',candidate))
            print(candidate)
            break
        except OSError:
            pass
PY")
    port=${port//$'\r'/}
  fi
  [[ "$port" =~ ^[0-9]+$ && "$port" -ge 1024 && "$port" -le 65535 ]] || die 'invalid port'
  # Docker rejects an occupied bind atomically. Never stop its current owner.
  dkr run -d --name "$container" --label notes.e2e=claude-plugin --cpus 2 --memory 2g \
    -p "$docker_ip:$port:8080" -v "$volume:/data" \
    -e NOTES_HUB_PUBLIC_URL="http://$docker_ip:$port" -e NOTES_HUB_AGENT_ENABLED=false \
    -e NOTES_HUB_TRUST_PROXY=false "$image" > "$run/container-id.txt"
  dkr container inspect "$container" > "$run/container.json"
else
  # Verify scope before reuse; do not replace another owner's resource.
  python - "$(cygpath -m "$run/container.json")" "$source_head" "$bubble_head" > "$run/container-runtime-head.txt" <<'PY'
import json,os,re,sys
c=json.load(open(sys.argv[1]))[0]
assert c['Config']['Image']=='notes-hub:e2e', 'unexpected container image'
recorded=c['Config'].get('Labels',{}).get('notes.e2e.source','')
assert re.fullmatch(r'[0-9a-f]{40}',recorded), 'missing or invalid container source commit'
assert c['Config'].get('Labels',{}).get('notes.e2e.bubblepaw')==sys.argv[3], 'container BubblePaw revision differs'
assert any(m.get('Name')=='notes-hub-e2e-data' and m.get('Destination')=='/data' for m in c['Mounts']), 'unexpected staging volume'
assert all(p.get('HostIp')==os.environ['E2E_DOCKER_IP'] for rows in c['HostConfig']['PortBindings'].values() for p in rows), 'unexpected host bind'
print(recorded)
PY
  recorded_source=$(cat "$run/container-runtime-head.txt")
  recorded_source=${recorded_source//$'\r'/}
  git cat-file -e "$recorded_source^{commit}" 2>/dev/null || die 'container source commit is unavailable locally; cannot verify build inputs'
  if ! git diff --quiet --no-ext-diff --no-textconv "$recorded_source" HEAD -- "${build_inputs[@]}"; then
    git diff --name-only --no-ext-diff --no-textconv "$recorded_source" HEAD -- "${build_inputs[@]}" > "$run/reuse-changed-build-inputs.txt"
    die 'container build inputs differ from HEAD; stop and review owned staging container'
  fi
  python - "$recorded_source" "$source_head" "$bubble_head" "${build_inputs[@]}" > "$run/reuse-source-validation.json" <<'PY'
import json,sys
print(json.dumps({'image_source_commit':sys.argv[1],'current_head':sys.argv[2],
                  'bubblepaw_commit':sys.argv[3],'build_inputs':sys.argv[4:],
                  'build_input_diff':'unchanged'},indent=2))
PY
  running=$(dkr inspect --format '{{.State.Running}}' "$container")
  running=${running//$'\r'/}
  [[ "$running" == true ]] || dkr start "$container" > "$run/container-start.txt"
fi
port=$(dkr inspect --format '{{(index (index .HostConfig.PortBindings "8080/tcp") 0).HostPort}}' "$container")
port=${port//$'\r'/}
export NOTES_HUB_URL="http://$docker_ip:$port"
printf '%s\n' "$NOTES_HUB_URL" > "$run/hub-url.txt"
healthy=0
for ((i=0;i<30;i++)); do
  if curl --fail --silent --max-time 3 "$NOTES_HUB_URL/health" > "$run/health.txt"; then healthy=1; break; fi
  sleep 1
done
(( healthy )) || die 'staging hub failed health check'

py private-file > "$run/token-file-acl.json"
if [[ ! -s "$token_file" ]]; then
  # Capture stdout directly into variables; values never occur in argv or logs.
  E2E_WRITE_TOKEN=$(dkr exec "$container" notes-hub --config /data/hub.toml --agent-token add e2e-claude --scope write 2> "$run/token-create-write.log")
  E2E_READ_TOKEN=$(dkr exec "$container" notes-hub --config /data/hub.toml --agent-token add e2e-read --scope read 2> "$run/token-create-read.log")
  E2E_WRITE_TOKEN=${E2E_WRITE_TOKEN//$'\r'/}; E2E_READ_TOKEN=${E2E_READ_TOKEN//$'\r'/}
  [[ "$E2E_WRITE_TOKEN" =~ ^[A-Za-z0-9_-]+$ && "$E2E_READ_TOKEN" =~ ^[A-Za-z0-9_-]+$ ]] || die 'unexpected token output (values withheld)'
  printf 'E2E_WRITE_TOKEN=%s\nE2E_READ_TOKEN=%s\n' "$E2E_WRITE_TOKEN" "$E2E_READ_TOKEN" > "$token_file"
  chmod 600 "$token_file"
else
  # Accept only the exact two simple assignments; never evaluate file contents.
  E2E_WRITE_TOKEN=$(sed -n 's/^E2E_WRITE_TOKEN=\([A-Za-z0-9_-]*\)$/\1/p' "$token_file")
  E2E_READ_TOKEN=$(sed -n 's/^E2E_READ_TOKEN=\([A-Za-z0-9_-]*\)$/\1/p' "$token_file")
  [[ -n "$E2E_WRITE_TOKEN" && -n "$E2E_READ_TOKEN" ]] || die 'invalid private token file'
  chmod 600 "$token_file"
fi
export E2E_WRITE_TOKEN E2E_READ_TOKEN NOTES_TOKEN="$E2E_WRITE_TOKEN"
# Install capture tooling only inside the owned staging container. All real
# hub requests, including import and plugin-defaults probes, are covered.
dkr exec -u 0 "$container" sh -c 'command -v tcpdump >/dev/null || (apt-get update -qq && apt-get install -y -qq tcpdump)' \
  2>&1 | sanitize > "$run/capture-install.log"
{
  # shellcheck disable=SC2016 # Record the capture PID inside the owned container.
  dkr exec -u 0 "$container" sh -c 'echo $$ > /tmp/notes-e2e-capture.pid; exec timeout -s INT 1800 tcpdump -i eth0 -s 0 -U -w - "tcp dst port 8080"' || {
    capture_status=$?
    [[ "$capture_status" == 124 || "$capture_status" == 130 ]]
  }
} 2> "$run/capture.stderr.txt" | py packets > "$run/claude-request-headers.jsonl" &
capture_pid=$!
capture_ready=0
for ((i=0;i<30;i++)); do
  if rg -q 'listening on eth0' "$run/capture.stderr.txt"; then capture_ready=1; break; fi
  sleep 1
done
(( capture_ready )) || die 'packet capture failed to start before hub requests'
py feedback-tools > "$run/feedback-tools.json"
"$notes" memory list 2>&1 | sanitize > "$run/memory-list-before.txt"
if (( ! skip_import )); then
  say 'Dry-run import, import, and unchanged second import'
  "$notes" memory import "$(cygpath -m "$projects")" --map "$(cygpath -m "$root/integrations/claude-plugin/import/claude-memory-map.json")" --dry-run --offline \
    2>&1 | sanitize > "$run/import-offline.txt"
  for stage in dry-run first second; do
    opts=(); [[ "$stage" != dry-run ]] || opts+=(--dry-run)
    "$notes" memory import "$(cygpath -m "$projects")" --map "$(cygpath -m "$root/integrations/claude-plugin/import/claude-memory-map.json")" "${opts[@]}" \
      2>&1 | sanitize > "$run/import-$stage.txt"
  done
  py import-check "$(cygpath -m "$run/import-second.txt")" > "$run/import-check.json"
  py import-counts "$(cygpath -m "$run/import-offline.txt")" "$(cygpath -m "$run/import-dry-run.txt")" \
    "$(cygpath -m "$run/import-first.txt")" "$(cygpath -m "$run/import-second.txt")" > "$run/import-counts.json"
fi
say 'Seeding one global memory for the index checks'
printf '%s\n' 'Temporary E2E seed for the global index checks.' |
  "$notes" memory write global/tools/e2e-seed --description 'Temporary E2E seed' --type reference 2>&1 | sanitize > "$run/seed-write.txt" ||
  die 'seeding the global memory failed'
"$notes" memory index --project global 2>&1 | sanitize > "$run/index.txt"
rg -q 'global/tools/e2e-seed' "$run/index.txt" || die 'global index has no seeded memory'
export E2E_MARKER="notes-plugin-e2e-$run_id"

run_claude() {
  local label=$1 prompt=$2
  # Pin every SessionStart hook to staging, including the userConfig-defaults
  # probe. NOTES_TOKEN selects write/read scope independently of MCP fallback.
  export CLAUDE_PLUGIN_OPTION_HUB_URL="$NOTES_HUB_URL" CLAUDE_PLUGIN_OPTION_HUB_TOKEN="$NOTES_TOKEN"
  E2E_HOOK_LOG=$(cygpath -m "$run/hook-$label.txt")
  export E2E_HOOK_LOG
  # SessionStart prefers LOCALAPPDATA under Git Bash, then XDG_CACHE_HOME.
  # Cover both branches so staging indexes stay in this run.
  local hook_cache="$run/hook-cache/$label"
  mkdir -p "$hook_cache/localappdata" "$hook_cache/xdg"
  local args=(-p "$prompt" --output-format stream-json --verbose --plugin-dir "$(cygpath -m "$plugin")"
    --setting-sources '' --tools ''
    --allowedTools mcp__plugin_notes_notes__search_notes mcp__plugin_notes_notes__memory_read mcp__plugin_notes_notes__memory_write
    --max-turns 8)
  [[ -z "${E2E_CLAUDE_MODEL:-}" ]] || args+=(--model "$E2E_CLAUDE_MODEL")
  LOCALAPPDATA="$(cygpath -m "$hook_cache/localappdata")" XDG_CACHE_HOME="$hook_cache/xdg" \
    timeout --foreground "${E2E_CLAUDE_TIMEOUT:-240}" claude "${args[@]}" \
    2> >(sanitize > "$run/claude-$label.stderr.txt") | sanitize > "$run/claude-$label.jsonl"
}
plugin="$run/plugin"
cp -R "$root/integrations/claude-plugin" "$plugin"
# Exclude this test harness from the loaded plugin copy.
rm -rf -- "$plugin/test"
py plugin "$(cygpath -m "$plugin")" defaults write
say 'Trying ephemeral userConfig defaults'
probe='Use only the notes plugin MCP tools. Call search_notes with query in:memory in:[[global]] E2E seed, then memory_read global/tools/e2e-seed. Report the path and whether a Memory index (notes vault) was provided at session start. Do not write anything.'
style=defaults
if ! run_claude probe-defaults "$probe" ||
   ! py stream-check "$(cygpath -m "$run/claude-probe-defaults.jsonl")" probe > "$run/probe-defaults-check.json" ||
   ! rg -q 'global/tools/e2e-seed' "$run/hook-probe-defaults.txt"; then
  if [[ -s "$run/probe-defaults-check.json" ]] &&
     rg -q '"connected": true' "$run/probe-defaults-check.json" &&
     rg -q 'global/tools/e2e-seed' "$run/hook-probe-defaults.txt"; then
    die 'defaults resolved but required tool calls failed; inspect evidence for model/plugin/tool bug'
  fi
  style=literal
  say 'userConfig defaults unavailable; using literal MCP config and hook env only in disposable copy'
  py plugin "$(cygpath -m "$plugin")" literal write
  run_claude probe-literal "$probe"
  py stream-check "$(cygpath -m "$run/claude-probe-literal.jsonl")" probe > "$run/probe-literal-check.json"
  rg -q 'global/tools/e2e-seed' "$run/hook-probe-literal.txt" || die 'SessionStart did not inject the seeded global memory index'
fi
printf '%s\n' "$style" > "$run/plugin-config-mode.txt"

say 'Running Claude write session and CLI marker readback'
run_claude write "Use only the notes plugin MCP tools. First call search_notes with query in:memory in:[[global]] E2E seed, then memory_read global/tools/e2e-seed. Then memory_write path global/tools/e2e-check, description Temporary isolated E2E check, type reference, name E2E check, and body exactly: $E2E_MARKER . Do not use Bash or filesystem tools. Report the read path and written path."
py stream-check "$(cygpath -m "$run/claude-write.jsonl")" write > "$run/write-check.json"
rg -q 'global/tools/e2e-seed' "$run/hook-write.txt" || die 'write SessionStart index missing'
"$notes" memory read global/tools/e2e-check 2>&1 | sanitize > "$run/marker-readback.txt"
rg -q -F "$E2E_MARKER" "$run/marker-readback.txt" || die 'CLI marker readback mismatch'
# Open a disposable document/log snapshot, never the live vault held by hub.
# Copying only core documents avoids token stores and SQLite lock contention.
# shellcheck disable=SC2016 # Snapshot paths and cleanup expand only in the owned container.
dkr exec -u 0 "$container" sh -c 'set -eu; command -v notes-e2e-inspector >/dev/null; snapshot=$(mktemp -d /tmp/notes-e2e-history.XXXXXX); trap '\''rm -rf "$snapshot"'\'' EXIT; mkdir -p "$snapshot/.vault"; cp -a /data/vault/.vault/docs /data/vault/.vault/logs "$snapshot/.vault/"; notes-e2e-inspector "$snapshot"' \
  2>&1 | sanitize > "$run/history-actor.json"

say 'Running Claude read-token session'
export NOTES_TOKEN="$E2E_READ_TOKEN"
py plugin "$(cygpath -m "$plugin")" "$style" read
run_claude read "Use only the notes plugin MCP tools. Call search_notes with query in:memory in:[[global]] $E2E_MARKER, then memory_read global/tools/e2e-check. Report its marker. If write tools are absent report that. If a write tool is listed, attempt memory_write at global/tools/e2e-check with body READ_TOKEN_MUST_NOT_WRITE, description read-scope check, type reference, and confirm refusal. Never use Bash or filesystem tools."
py stream-check "$(cygpath -m "$run/claude-read.jsonl")" read > "$run/read-check.json"
"$notes" memory read global/tools/e2e-check 2>&1 | sanitize > "$run/marker-after-read.txt"
rg -q -F "$E2E_MARKER" "$run/marker-after-read.txt" || die 'read token changed marker'
! rg -q 'READ_TOKEN_MUST_NOT_WRITE' "$run/marker-after-read.txt" || die 'read token write unexpectedly succeeded'
stop_capture || die 'packet capture did not verify bearer requests without Origin'

# Independent negative check after capture (the deliberate Origin is excluded
# from actual-Claude no-Origin counts). No bearer credential is sent.
status=$(curl --silent --max-time 10 -o "$run/origin-refusal.txt" -w '%{http_code}' \
  -H 'Origin: https://untrusted.example' "$NOTES_HUB_URL/mcp")
printf '%s\n' "$status" > "$run/origin-status.txt"
[[ "$status" == 403 ]] || die 'Origin-only MCP request was not 403'
all_checks_passed=1
