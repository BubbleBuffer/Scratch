// The Sync and devices settings page and the "Add a device" flow.
//
// - Status card: whether this vault syncs, with "Sync now".
// - Your hub: sign in to a hub (or sign out), or connect with a pairing code
//   (collapsed): a pairing string, or a hub address and a token; then the
//   background loop runs until "Disconnect".
// - Attachments: background download, or "Download all" now.
// - This computer as a hub (desktop): on/off, the addresses other devices can
//   use, the devices paired to it with "Add a device" (a token for a named
//   device, shown as text and as a QR code of the pairing string), and the
//   listen address (collapsed).
// - Trusted certificates: hubs whose unknown-issuer certificate this device
//   trusts, each viewable and revocable.

import 'package:flutter/widgets.dart';

import '../../state/app_state.dart';
import '../../src/rust/api/sync.dart' show redeemPairingCode;
import '../../state/sync_store.dart';
import '../../ui/app_name.dart';
import '../../ui/ui.dart';
import '../settings/controls.dart';
import '../settings/settings_kit.dart';
import '../settings/settings_screen.dart' show pushSettingsPage;
import '../settings/settings_store.dart';
import 'cert_trust.dart';
import 'certificate_view.dart';
import 'pairing_codec.dart';
import 'hub_auth.dart';
import 'qr.dart';
import 'sync_control.dart';
import 'sync_indicator.dart';

// ---------------------------------------------------------------------------
// The page

/// The whole page. [extra] are add-on sections placed with the hub ("Run
/// schedules on this device").
class SyncAndDevicesPage extends StatelessWidget {
  const SyncAndDevicesPage({super.key, this.extra = const []});
  final List<Widget> extra;

  @override
  Widget build(BuildContext context) {
    final sync = context.watch<SyncStore>();
    final s = AppSettings.of(context);
    final signedIn =
        s.clientToken != null && s.clientUrl != null && s.clientDeviceLabel != null;
    return SPage(
      title: 'Sync and devices',
      intro:
          'Keep this vault the same on all your devices. A **hub** is the computer or server your devices sync through.',
      children: [
        const SyncStatusCard(),
        SSection(
          title: 'Your hub',
          children: [
            if (sync.connected && !signedIn)
              const HubConnectionRow()
            else
              const HubSignInSettings(),
            if (!signedIn && !sync.connected) const ConnectSettings(),
          ],
        ),
        const SSection(title: 'Attachments', children: [SBlock(child: AttachmentSettings())]),
        if (SyncStore.canHost)
          SSection(
            title: 'This computer as a hub',
            intro: 'Let your other devices sync with this computer directly, without a server.',
            children: [HubSettings(extra: extra)],
          )
        else if (extra.isNotEmpty)
          SSection(children: extra),
        const TrustedCertificatesSettings(),
      ],
    );
  }
}

/// Whether the vault syncs, and with whom; "Sync now" while connected. The
/// error of a failed sync, or of an unknown certificate, comes under it.
class SyncStatusCard extends StatelessWidget {
  const SyncStatusCard({super.key});

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final sync = context.watch<SyncStore>();
    if (!sync.connected) {
      return const SStatusCard(
        icon: Sx.cloudOff,
        title: 'Not syncing',
        text: 'This vault lives only on this device. Connect a hub to use it on your other devices.',
        ok: false,
      );
    }
    final (phase, message) = sync.phase;
    final ok = sync.status.lastOkMs;
    final good = phase == SyncPhase.synced || phase == SyncPhase.syncing;
    return Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
      SStatusCard(
        icon: switch (phase) {
          SyncPhase.synced => Sx.check,
          SyncPhase.syncing => Sx.sync,
          SyncPhase.offline => Sx.cloudOff,
          _ => Sx.alert,
        },
        title: phaseLabel(phase),
        text: [
          'With ${sync.hubUrl}',
          if (ok != null) 'last synced ${relativeTime(ok.toInt()).toLowerCase()}',
        ].join(' · '),
        ok: good,
        action: SButton('Sync now', key: const ValueKey('sync-now-button'), icon: Sx.sync, onTap: sync.syncNow),
      ),
      if (sync.untrustedCertificate)
        const Padding(
          padding: EdgeInsets.only(top: 12),
          child: SyncCertificateProblem(messageKey: ValueKey('connect-error')),
        )
      else if (message != null)
        Padding(
          padding: const EdgeInsets.only(top: 12),
          child: Text(message, key: const ValueKey('connect-error'), style: tsRaw(13, 18, 400, c.dangerText)),
        ),
    ]);
  }
}

/// Connected by a pairing code, so there is no account to sign out of.
class HubConnectionRow extends StatelessWidget {
  const HubConnectionRow({super.key});

  @override
  Widget build(BuildContext context) {
    final sync = context.watch<SyncStore>();
    final s = AppSettings.of(context);
    return SRow(
      label: 'Hub connection',
      desc: 'Connected to ${sync.hubUrl}.',
      help: 'Disconnect to sync with another hub. Your pages stay on this device.',
      control: SButton(
        'Disconnect',
        key: const ValueKey('disconnect-button'),
        kind: SButtonKind.danger,
        onTap: () => disconnectFromHub(sync, s),
      ),
    );
  }
}

// ---------------------------------------------------------------------------
// Your hub

class HubSignInSettings extends StatefulWidget {
  const HubSignInSettings({super.key, this.auth});

  /// Supplies a browser login in tests.
  final HubAuthService? auth;

  @override
  State<HubSignInSettings> createState() => _HubSignInSettingsState();
}

class _HubSignInSettingsState extends State<HubSignInSettings> {
  // The address is the device's own setting; it survives sign-out. It is set
  // in initState: a lazy initializer would first read the context in dispose
  // when the field is never built (signed in).
  final _url = TextEditingController();
  late final _auth = widget.auth ?? HubAuthService();
  bool _waiting = false;
  bool _signingOut = false;
  bool _cancelled = false;
  String? _error;

  @override
  void initState() {
    super.initState();
    _url.text = AppSettings.read(context).hubSignInUrl;
  }

  @override
  void dispose() {
    _auth.dispose();
    _url.dispose();
    super.dispose();
  }

  Future<void> _signIn() async {
    final sync = context.read<SyncStore>();
    final settings = AppSettings.read(context);
    setState(() {
      _waiting = true;
      _cancelled = false;
      _error = null;
    });
    try {
      await signInToHub(sync, settings, _auth, _url.text.trim());
    } catch (e) {
      if (!_cancelled && mounted) {
        _error = e is HubAuthException
            ? e.message
            : 'Could not start sync with the hub.';
      }
    }
    if (mounted) setState(() => _waiting = false);
  }

  void _cancel() {
    _cancelled = true;
    _auth.cancel();
  }

  Future<void> _signOut() async {
    final sync = context.read<SyncStore>();
    final settings = AppSettings.read(context);
    setState(() {
      _signingOut = true;
      _error = null;
    });
    try {
      await signOutFromHub(sync, settings, _auth);
    } catch (_) {
      if (mounted) {
        _error = 'Signed out on this device; the hub could not be reached.';
      }
    }
    if (mounted) setState(() => _signingOut = false);
  }

  @override
  Widget build(BuildContext context) {
    final s = AppSettings.of(context);
    final signedIn =
        s.clientToken != null &&
        s.clientUrl != null &&
        s.clientDeviceLabel != null;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        if (signedIn)
          SRow(
            label: 'Hub account',
            // An older hub reports no account: then the device name it
            // registered this device under is shown, never a made-up account.
            desc:
                'Signed in to ${Uri.tryParse(s.clientUrl!)?.host ?? s.clientUrl} as ${s.clientAccount ?? s.clientDeviceLabel}',
            help: 'Sign out to switch hubs or accounts. Your pages stay on this device.',
            control: SButton(
              _signingOut ? 'Signing out…' : 'Sign out',
              key: const ValueKey('hub-sign-out'),
              onTap: _signingOut ? null : _signOut,
            ),
          )
        else if (_waiting)
          SRow(
            label: 'Waiting for the browser…',
            desc: 'Finish signing in there. This page updates when you are done.',
            control: SButton(
              'Cancel',
              key: const ValueKey('hub-sign-in-cancel'),
              onTap: _cancel,
            ),
          )
        else
          SRow(
            label: 'Sign in to your hub',
            desc: 'Your hub’s web address. $appName opens your browser to sign in.',
            help:
                'Use this if your hub runs on a server, such as one you host or one you were invited to.',
            stack: true,
            control: Row(children: [
              Expanded(
                child: STextField(
                  fieldKey: const ValueKey('hub-sign-in-url'),
                  controller: _url,
                  placeholder: 'https://notes.example.com',
                  onChanged: (v) => AppSettings.read(context).update(
                    (s) => s.hubSignInUrl = v.trim(),
                  ),
                  onSubmitted: (_) => _signIn(),
                ),
              ),
              const SizedBox(width: 8),
              SButton('Sign in', key: const ValueKey('hub-sign-in'), kind: SButtonKind.primary, onTap: _signIn),
            ]),
          ),
        if (_error != null)
          SNotice(_error!, kind: SNoticeKind.bad, textKey: const ValueKey('hub-sign-in-error')),
      ],
    );
  }
}

/// Connect with a pairing code (collapsed): for a hub on another of this
/// person's computers.
class ConnectSettings extends StatefulWidget {
  const ConnectSettings({super.key});

  @override
  State<ConnectSettings> createState() => _ConnectSettingsState();
}

class _ConnectSettingsState extends State<ConnectSettings> {
  final _hub = TextEditingController();
  final _token = TextEditingController();
  String? _error;

  /// The hub of the last failed connect (for an untrusted certificate).
  String? _failedUrl;

  /// The one-time code of the last pasted code link (the hub field then
  /// keeps only its address); redeemed on connect unless a token is typed.
  String? _pastedCode;
  bool _busy = false;

  @override
  void dispose() {
    _hub.dispose();
    _token.dispose();
    super.dispose();
  }

  /// A pasted pairing string fills both fields.
  void _hubChanged(String v) {
    final p = Pairing.parse(v);
    if (p != null) {
      _pastedCode = p.code;
      _hub.text = p.url;
      _token.text = p.token;
    }
    if (_error != null) setState(() => _error = null);
  }

  Future<void> _connect() async {
    final p = Pairing.parse(_hub.text);
    final url = p?.url ?? normalizeHubUrl(_hub.text);
    final typed = p?.token ?? _token.text.trim();
    final code = p?.code ?? (typed.isEmpty ? _pastedCode : null);
    if (url.isEmpty || (code == null && typed.isEmpty)) {
      setState(() => _error = 'Enter the hub address and the token, or paste a pairing code.');
      return;
    }
    setState(() => _busy = true);
    try {
      final sync = context.read<SyncStore>();
      final settings = AppSettings.read(context);
      // A code link is redeemed for the device's token; an older link has it.
      final token = code != null
          ? await redeemPairingCode(
              vault: sync.vault.raw,
              hubUrl: url,
              code: code,
              trusted: trustedFor(settings, url),
            )
          : typed;
      await connectToHub(sync, settings, url, token);
      _hub.clear();
      _token.clear();
      _error = null;
    } catch (e) {
      _error = '$e';
      _failedUrl = url;
    }
    if (mounted) setState(() => _busy = false);
  }

  /// The chain of a failed first connect: the client's, or only the
  /// fingerprint the error named.
  List<CertDetails> _failedChain() {
    final sync = context.read<SyncStore>();
    if (sync.hubUrl == _failedUrl && sync.untrustedChain != null) return syncCertificateChain(sync);
    final fp = untrustedFingerprintOf(_error);
    return fp == null ? const [] : [CertDetails(sha256: fp)];
  }

  /// Confirms, trusts the certificate of the failed connect, and connects
  /// again with the same address and token.
  Future<bool> _trustFailed(BuildContext context) async {
    final url = _failedUrl, chain = _failedChain();
    if (url == null || chain.isEmpty) return false;
    final fp = untrustedFingerprintOf(_error) ?? chain.first.sha256;
    final sync = context.read<SyncStore>();
    final issuerDer = sync.hubUrl == url && sync.untrustedChain != null ? sync.untrustedIssuerDer : null;
    if (!await confirmTrustCertificate(context, host: serverHostOf(url), fingerprint: fp, issuer: chain.first.issuer)) {
      return false;
    }
    if (!mounted) return false;
    await trustCertificate(this.context.read<SyncStore>(), AppSettings.read(this.context), url, chain,
        fingerprint: fp, issuerDer: issuerDer);
    if (mounted) await _connect();
    return true;
  }

  Widget _failedCertificate(BuildContext context) {
    final url = _failedUrl ?? '';
    final changed = AppSettings.of(context).trustedCertificates.changed(url, untrustedFingerprintOf(_error));
    return CertificateProblem(
      host: serverHostOf(url),
      changed: changed,
      onView: () {
        final chain = _failedChain();
        if (chain.isEmpty) return;
        showCertificate(
          context,
          host: serverHostOf(url),
          status: changed ? CertStatus.changed : CertStatus.untrusted,
          chain: chain,
          onTrust: _trustFailed,
        );
      },
      onTrust: () => _trustFailed(context),
    );
  }

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    return SAdvanced(
      headerKey: const ValueKey('connect-advanced'),
      title: 'Connect with a pairing code',
      summary: 'for a hub on another of your computers',
      initiallyOpen: _error != null,
      children: [
        SRow(
          label: 'Pairing code or hub address',
          desc: SyncStore.canHost
              ? 'On the computer that runs the hub, open Settings › Sync and devices › Add a device, then paste the code here.'
              : 'On the computer that runs the hub, open Settings › Sync and devices › Add a device, then paste the pairing code here.',
          stack: true,
          control: Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
            STextField(
              fieldKey: const ValueKey('connect-hub-field'),
              controller: _hub,
              mono: true,
              placeholder: 'notes://pair?… or http://100.101.102.103:47100',
              onChanged: _hubChanged,
            ),
            SField(
              label: 'Token',
              hint: 'Pasting a pairing code fills both fields.',
              child: STextField(
                fieldKey: const ValueKey('connect-token-field'),
                controller: _token,
                mono: true,
                placeholder: 'Filled in by the pairing code',
                onSubmitted: (_) => _connect(),
              ),
            ),
            if (_error != null)
              Padding(
                padding: const EdgeInsets.only(top: 10),
                child: isUntrustedCertificateError(_error)
                    ? _failedCertificate(context)
                    : Text(_error!, style: tsRaw(13, 18, 400, c.dangerText)),
              ),
            const SizedBox(height: 10),
            Align(
              alignment: Alignment.centerLeft,
              child: SButton(
                _busy ? 'Connecting…' : 'Connect',
                key: const ValueKey('connect-button'),
                onTap: _busy ? null : _connect,
              ),
            ),
          ]),
        ),
      ],
    );
  }
}

// ---------------------------------------------------------------------------
// Attachments

class AttachmentSettings extends StatelessWidget {
  const AttachmentSettings({super.key});

  @override
  Widget build(BuildContext context) {
    final sync = context.watch<SyncStore>();
    final s = AppSettings.of(context);
    final progress = sync.attachmentProgress;
    return Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
      SRow(
        label: 'Download attachments in the background',
        desc: 'Otherwise files and images download when a page shows them.',
        help:
            'Turn on to have every file available offline. Leave off on a phone with little storage or a metered connection.',
        control: SSwitch(
          key: const ValueKey('prefetch-toggle'),
          label: 'Download attachments in the background',
          value: s.prefetchAttachments,
          onChanged: (v) => setPrefetch(sync, s, v),
        ),
      ),
      const SRule(),
      SRow(
        label: 'Download all attachments now',
        desc: progress != null
            ? 'Downloading ${progress.$1} of ${progress.$2}…'
            : sync.connected
                ? 'Get every file once, to have them offline.'
                : 'Connect to a hub first.',
        control: SButton(
          'Download',
          key: const ValueKey('download-attachments'),
          onTap: sync.connected && progress == null ? () => sync.fetchAllAttachments() : null,
        ),
      ),
    ]);
  }
}

// ---------------------------------------------------------------------------
// This computer as a hub

class HubSettings extends StatefulWidget {
  const HubSettings({super.key, this.extra = const []});

  /// Rows placed after the hub's own ("Run schedules on this device").
  final List<Widget> extra;

  @override
  State<HubSettings> createState() => _HubSettingsState();
}

class _HubSettingsState extends State<HubSettings> {
  late final _addr = TextEditingController(text: AppSettings.read(context).hubAddress);
  List<(String, String)> _urls = const [];
  String? _urlsFor;

  @override
  void dispose() {
    _addr.dispose();
    super.dispose();
  }

  Future<void> _loadUrls(String bound) async {
    _urlsFor = bound;
    final u = await hubUrls(bound);
    if (mounted && _urlsFor == bound) setState(() => _urls = u);
  }

  @override
  Widget build(BuildContext context) {
    final sync = context.watch<SyncStore>();
    final s = AppSettings.of(context);
    final bound = sync.hubAddress;
    if (bound != null && bound != _urlsFor) _loadUrls(bound);
    return Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
      SRow(
        label: 'Share this vault with your devices',
        desc:
            'This computer runs a hub while $appName is open. Your phone syncs with it over your network or Tailscale.',
        help:
            'Turn on if you have no hub on a server. Turn off if you sync through a hosted hub and don’t need this computer to serve.',
        control: SSwitch(
          key: const ValueKey('hub-toggle'),
          label: 'Share this vault with your devices',
          value: s.hubEnabled,
          onChanged: (v) => setHubEnabled(sync, s, v),
        ),
      ),
      if (sync.hubError != null)
        SNotice(sync.hubError!, kind: SNoticeKind.bad, textKey: const ValueKey('hub-error')),
      if (bound != null) ...[
        const SRule(),
        SRow(
          label: 'Reachable at',
          desc: 'Devices can use any of these addresses.',
          stack: true,
          control: Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
            for (final (url, _) in _urls) ...[
              SCopyLine(url),
              const SizedBox(height: 6),
            ],
          ]),
        ),
        const SRule(),
        const SRow(
          label: 'Paired devices',
          desc: 'Unpairing revokes a device’s access; it stops syncing.',
          stack: true,
          control: PairedDevices(),
        ),
        SAdvanced(
          title: 'Advanced',
          summary: 'network address',
          children: [
            SRow(
              label: 'Listen address',
              desc: 'Which network connections the hub accepts, and on which port.',
              help: 'Only if a firewall or another program needs a different port.',
              extra:
                  '**0.0.0.0** accepts connections from every network this computer is on. A port of 0 picks a free one.',
              stack: true,
              control: Row(children: [
                Expanded(
                  child: STextField(
                    fieldKey: const ValueKey('hub-address-field'),
                    controller: _addr,
                    mono: true,
                    onSubmitted: (v) => setHubAddress(sync, s, v.trim()),
                  ),
                ),
                const SizedBox(width: 8),
                SButton('Apply', key: const ValueKey('hub-address-apply'), onTap: () => setHubAddress(sync, s, _addr.text.trim())),
              ]),
            ),
          ],
        ),
      ],
      for (final e in widget.extra) ...[
        const SRule(),
        e,
      ],
    ]);
  }
}

/// The devices paired with this computer's hub, with "Add a device".
class PairedDevices extends StatefulWidget {
  const PairedDevices({super.key});

  @override
  State<PairedDevices> createState() => _PairedDevicesState();
}

class _PairedDevicesState extends State<PairedDevices> {
  @override
  void initState() {
    super.initState();
    context.read<SyncStore>().refreshDevices();
  }

  Future<void> _unpair(String label) async {
    final sync = context.read<SyncStore>();
    await sync.unpair(label);
  }

  @override
  Widget build(BuildContext context) {
    final sync = context.watch<SyncStore>();
    final add = SButton(
      'Add a device',
      key: const ValueKey('devices-add'),
      kind: sync.devices.isEmpty ? SButtonKind.primary : SButtonKind.normal,
      icon: Sx.plus,
      onTap: () => showAddDevice(context),
    );
    if (sync.devices.isEmpty) {
      return SEmpty(
        icon: Sx.phone,
        title: 'No devices paired yet',
        text:
            'Pair your phone or another computer to sync with this one. You’ll get a code to scan or paste.',
        action: add,
      );
    }
    return SList(
      foot: SListFoot(children: [add]),
      children: [
        for (final d in sync.devices)
          KeyedSubtree(
            key: ValueKey('device-${d.label}'),
            child: SItem(
              icon: Sx.phone,
              name: d.label,
              status: d.device == null
                  ? const SStatus('Waiting')
                  : const SStatus('Paired', kind: SStatusKind.ok),
              sub: d.device == null
                  ? 'Paired ${relativeTime(d.createdMs.toInt()).toLowerCase()}, not connected yet'
                  : 'Device ${d.device!.substring(0, d.device!.length.clamp(0, 8))} · paired ${relativeTime(d.createdMs.toInt()).toLowerCase()}',
              trailing: [
                SButton(
                  'Unpair',
                  key: ValueKey('unpair-${d.label}'),
                  kind: SButtonKind.danger,
                  onTap: () => _unpair(d.label),
                ),
              ],
            ),
          ),
      ],
    );
  }
}

// ---------------------------------------------------------------------------
// Trusted certificates

/// The certificates this device trusts for hubs with an unknown issuer, one
/// row per server; shown only when there are any.
class TrustedCertificatesSettings extends StatelessWidget {
  const TrustedCertificatesSettings({super.key});

  Future<void> _revoke(BuildContext context, String origin) async {
    final sync = context.read<SyncStore>();
    final s = AppSettings.read(context);
    if (!await confirmRevokeCertificate(context, serverHostOf(origin))) return;
    await revokeCertificate(sync, s, origin);
  }

  void _open(BuildContext context, String origin, PinnedCertificate pin) {
    final sync = context.read<SyncStore>();
    final current = sync.hubUrl;
    // The hub now presents another certificate: show that one, and its trust.
    if (current != null && originOf(current) == origin && sync.untrustedCertificate) {
      showSyncCertificate(context);
      return;
    }
    showCertificate(
      context,
      host: serverHostOf(origin),
      status: CertStatus.trusted,
      chain: pin.viewChain,
      trustedAtMs: pin.trustedAtMs,
      onRevoke: (ctx) async {
        if (!await confirmRevokeCertificate(ctx, serverHostOf(origin))) return false;
        if (!ctx.mounted) return false;
        await revokeCertificate(ctx.read<SyncStore>(), AppSettings.read(ctx), origin);
        return true;
      },
    );
  }

  /// The row's text: the issuing CA's CN, its O · OU, then `for HOST ·
  /// FINGERPRINT · valid until …`. An issuer without a CN keeps the
  /// host as the title.
  Widget _pinSummary(BuildContext context, String origin, PinnedCertificate pin) {
    final c = context.colors;
    final m = context.mobile;
    final host = serverHostOf(origin);
    final name = pin.issuerName;
    final cn = name.cn;
    final org = [?name.o, ?name.ou].join(' · ');
    return Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
      Text(cn ?? host,
          key: ValueKey('cert-pin-title-$origin'), style: tsRaw(m ? 15 : 14, 19, 680, c.textPrimary)),
      if (cn != null && org.isNotEmpty) ...[
        const SizedBox(height: 2),
        Text(org, key: ValueKey('cert-pin-org-$origin'), style: sHint(context)),
      ],
      const SizedBox(height: 2),
      Text.rich(
        key: ValueKey('cert-pin-detail-$origin'),
        TextSpan(children: [
          if (cn != null) TextSpan(text: 'for $host · '),
          TextSpan(text: shortFingerprint(pin.sha256), style: sMono(context, color: c.textTertiary)),
          if (pin.notAfterMs > 0) TextSpan(text: ' · valid until ${formatCertDate(pin.notAfterMs)}'),
        ]),
        style: sHint(context),
      ),
    ]);
  }

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final pins = AppSettings.of(context).trustedCertificates;
    if (pins.isEmpty) return const SizedBox.shrink();
    return SAdvanced(
      headerKey: const ValueKey('trusted-certificates'),
      title: 'Trusted certificates',
      summary: pins.length == 1 ? '1 certificate' : '${pins.length} certificates',
      children: [
        SBlock(
          child: Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
            Padding(
              padding: const EdgeInsets.only(top: 4, bottom: 10),
              child: Text(
                'Hub certificates with an issuer this device doesn’t know, trusted on this device only.',
                style: sDesc(context),
              ),
            ),
            SList(children: [
              for (final (origin, pin) in pins.entries)
                Padding(
                  key: ValueKey('cert-pin-$origin'),
                  padding: const EdgeInsets.fromLTRB(12, 8, 8, 8),
                  child: Row(children: [
                    SIcon(Sx.shieldCheck, size: 17, color: c.textSecondary),
                    const SizedBox(width: 12),
                    Expanded(
                      child: Hover(
                        key: ValueKey('cert-pin-open-$origin'),
                        onTap: () => _open(context, origin, pin),
                        radius: Radii.md,
                        padding: const EdgeInsets.symmetric(horizontal: 4, vertical: 4),
                        child: _pinSummary(context, origin, pin),
                      ),
                    ),
                    const SizedBox(width: 8),
                    SButton(
                      'Revoke',
                      key: ValueKey('cert-pin-revoke-$origin'),
                      kind: SButtonKind.danger,
                      onTap: () => _revoke(context, origin),
                    ),
                  ]),
                ),
            ]),
          ]),
        ),
      ],
    );
  }
}

// ---------------------------------------------------------------------------
// Add a device

/// Opens "Add a device"; [token] skips the naming step (screenshots).
Future<void> showAddDevice(BuildContext context, {String? token}) {
  if (context.mobile) return pushSettingsPage(context, 'Add a device', AddDeviceView(initialToken: token));
  return showPanel(
    context,
    (_) => SingleChildScrollView(
        padding: const EdgeInsets.fromLTRB(40, 32, 40, 32), child: AddDeviceView(initialToken: token)),
    width: 560,
    height: 760,
    shrink: true,
  );
}

class AddDeviceView extends StatefulWidget {
  const AddDeviceView({super.key, this.initialToken});

  /// Skips the name step with this token (screenshots).
  final String? initialToken;

  @override
  State<AddDeviceView> createState() => _AddDeviceViewState();
}

class _AddDeviceViewState extends State<AddDeviceView> {
  final _name = TextEditingController(text: 'Phone');
  late String? _token = widget.initialToken;
  List<(String, String)> _urls = const [];
  String? _url;
  String? _error;

  @override
  void initState() {
    super.initState();
    _loadUrls();
  }

  @override
  void dispose() {
    _name.dispose();
    super.dispose();
  }

  Future<void> _loadUrls() async {
    final sync = context.read<SyncStore>();
    final addr = sync.hubAddress ?? AppSettings.read(context).hubAddress;
    final u = await hubUrls(addr);
    if (!mounted) return;
    setState(() {
      _urls = u;
      _url ??= u.isEmpty ? null : u.first.$1;
    });
  }

  Future<void> _create() async {
    final label = _name.text.trim();
    if (label.isEmpty) return;
    final sync = context.read<SyncStore>();
    if (sync.devices.any((d) => d.label == label)) {
      setState(() => _error = 'A device named "$label" is already paired. Pick another name, or unpair it first.');
      return;
    }
    try {
      final t = await sync.pair(label);
      if (mounted) {
        setState(() {
          _token = t;
          _error = null;
        });
      }
    } catch (e) {
      if (mounted) setState(() => _error = '$e');
    }
  }

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final m = context.mobile;
    final sync = context.watch<SyncStore>();
    final body = m ? tsRaw(15, 21, 400, c.textSecondary) : tsRaw(14, 20, 400, c.textSecondary);
    final token = _token;
    final children = <Widget>[
      if (!m) ...[
        Text('Add a device', style: tsRaw(20, 26, 600, c.textPrimary)),
        const SizedBox(height: 8),
      ],
    ];
    if (token == null) {
      children.addAll([
        Text('Name the device, then scan the code with it or copy the pairing code to it.', style: body),
        const SizedBox(height: 16),
        SettingsField(
          fieldKey: const ValueKey('device-name-field'),
          controller: _name,
          placeholder: 'Phone',
          autofocus: !m,
          onSubmitted: (_) => _create(),
        ),
        if (_error != null) ...[
          const SizedBox(height: 8),
          Text(_error!, style: tsRaw(13, 18, 400, c.inlineCodeText)),
        ],
        const SizedBox(height: 16),
        Row(children: [PrimaryButton(key: const ValueKey('create-pairing'), label: 'Create pairing code', onTap: _create)]),
      ]);
    } else {
      final url = _url ?? 'http://127.0.0.1:47100';
      final code = Pairing.code(url, token).encode();
      children.addAll([
        Text('On the device, open $appName › Settings › Sync and devices › Connect with a pairing code, and scan or paste this code. '
            'The token works for one device and is shown only now.', style: body),
        const SizedBox(height: 20),
        Center(
          child: Container(
            key: const ValueKey('pairing-qr'),
            padding: const EdgeInsets.all(4),
            decoration: BoxDecoration(
              color: const Color(0xFFFFFFFF),
              borderRadius: BorderRadius.circular(Radii.md),
              border: Border.all(color: c.border),
            ),
            child: QrView(code: QrCode.encode(code), size: m ? 220 : 200),
          ),
        ),
        const SizedBox(height: 20),
        if (!sync.hosting)
          Padding(
            padding: const EdgeInsets.only(bottom: 12),
            child: Text('The hub is off. Turn it on in Sync and devices so the device can connect.',
                style: tsRaw(13, 18, 400, c.inlineCodeText)),
          ),
        Row(children: [
          Expanded(child: Text('Hub address', style: tsRaw(12, 16, 500, c.textSecondary))),
          if (_urls.length > 1)
            SelectButton<String>(
              key: const ValueKey('pair-url-select'),
              title: 'Hub address',
              value: url,
              options: [for (final u in _urls) u.$1],
              labelOf: (u) => _urls.firstWhere((x) => x.$1 == u, orElse: () => (u, '')).$2,
              onChanged: (u) => setState(() => _url = u),
            ),
        ]),
        const SizedBox(height: 6),
        CopyableText(url, textKey: const ValueKey('pair-url')),
        const SizedBox(height: 12),
        Text('Token', style: tsRaw(12, 16, 500, c.textSecondary)),
        const SizedBox(height: 6),
        CopyableText(token, textKey: const ValueKey('pair-token')),
        const SizedBox(height: 12),
        Text('Pairing code (address and token)', style: tsRaw(12, 16, 500, c.textSecondary)),
        const SizedBox(height: 6),
        CopyableText(code, textKey: const ValueKey('pair-code')),
        const SizedBox(height: 20),
        Row(children: [
          PrimaryButton(key: const ValueKey('pair-done'), label: 'Done', onTap: () => Navigator.of(context).maybePop()),
        ]),
      ]);
    }
    return Column(crossAxisAlignment: CrossAxisAlignment.stretch, mainAxisSize: MainAxisSize.min, children: children);
  }
}
