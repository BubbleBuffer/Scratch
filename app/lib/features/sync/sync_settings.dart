// The Sync and Devices settings sections and the "Add a device" flow.
//
// - Hub (desktop): on/off, listen address, the addresses other devices can
//   use, and pairing: a token for a named device, shown as text and as a QR
//   code of the pairing string.
// - Connect to a hub (any device): a pairing string, or a hub address and
//   a token; then the background loop runs until "Disconnect".
// - Attachments: background download, or "Download all" now.
// - Trusted certificates: hubs whose unknown-issuer certificate this device
//   trusts, each viewable and revocable.

import 'package:flutter/widgets.dart';

import '../../state/app_state.dart';
import '../../state/sync_store.dart';
import '../../ui/app_name.dart';
import '../../ui/ui.dart';
import '../settings/controls.dart';
import '../settings/settings_screen.dart';
import '../settings/settings_store.dart';
import 'cert_trust.dart';
import 'certificate_view.dart';
import 'pairing_codec.dart';
import 'hub_auth.dart';
import 'qr.dart';
import 'sync_control.dart';
import 'sync_icons.dart';
import 'sync_indicator.dart';

// ---------------------------------------------------------------------------
// Sync

class SyncBody extends StatelessWidget {
  const SyncBody({super.key});

  @override
  Widget build(BuildContext context) {
    return Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
      const SettingsHeading('Sync', primary: true),
      const SettingsHeading('Hub'),
      const HubSignInSettings(),
      const SizedBox(height: 28),
      if (SyncStore.canHost) ...[
        const HubSettings(),
        const SizedBox(height: 28),
        const SettingsHeading('Connect to a hub'),
      ],
      const ConnectSettings(),
      const TrustedCertificatesSettings(),
      const SizedBox(height: 28),
      const SettingsHeading('Attachments'),
      const AttachmentSettings(),
    ]);
  }
}

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
    final c = context.colors;
    final s = AppSettings.of(context);
    final signedIn =
        s.clientToken != null &&
        s.clientUrl != null &&
        s.clientDeviceLabel != null;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        if (signedIn)
          SettingsRow(
            // An older hub reports no account: then the device name it
            // registered this device under is shown, never a made-up account.
            label:
                'Signed in to ${Uri.tryParse(s.clientUrl!)?.host ?? s.clientUrl} as ${s.clientAccount ?? s.clientDeviceLabel}',
            stack: context.mobile,
            trailing: OutlineButton(
              key: const ValueKey('hub-sign-out'),
              label: _signingOut ? 'Signing out…' : 'Sign out',
              onTap: _signingOut ? null : _signOut,
            ),
          )
        else if (_waiting)
          SettingsRow(
            label: 'Waiting for the browser…',
            trailing: OutlineButton(
              key: const ValueKey('hub-sign-in-cancel'),
              label: 'Cancel',
              onTap: _cancel,
            ),
          )
        else ...[
          SettingsField(
            fieldKey: const ValueKey('hub-sign-in-url'),
            controller: _url,
            placeholder: 'https://notes.breadcat.cc',
            onChanged: (v) => AppSettings.read(context).update(
              (s) => s.hubSignInUrl = v.trim(),
            ),
            onSubmitted: (_) => _signIn(),
          ),
          const SizedBox(height: 14),
          Row(
            children: [
              PrimaryButton(
                key: const ValueKey('hub-sign-in'),
                label: 'Sign in',
                onTap: _signIn,
              ),
            ],
          ),
        ],
        if (_error != null)
          Padding(
            padding: const EdgeInsets.only(top: 8),
            child: Text(
              _error!,
              key: const ValueKey('hub-sign-in-error'),
              style: tsRaw(13, 18, 400, c.inlineCodeText),
            ),
          ),
      ],
    );
  }
}

class HubSettings extends StatefulWidget {
  const HubSettings({super.key});

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
    final c = context.colors;
    final sync = context.watch<SyncStore>();
    final s = AppSettings.of(context);
    final bound = sync.hubAddress;
    if (bound != null && bound != _urlsFor) _loadUrls(bound);
    return Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
      SettingsRow(
        label: 'Share this vault with your devices',
        description: 'This computer runs the hub. Your phone syncs with it over your network or Tailscale.',
        trailing: Toggle(
          key: const ValueKey('hub-toggle'),
          value: s.hubEnabled,
          onChanged: (v) => setHubEnabled(sync, s, v),
        ),
      ),
      SettingsRow(
        label: 'Listen address',
        description: '0.0.0.0 accepts every network; a port of 0 picks a free one.',
        trailing: SizedBox(
          width: 220,
          child: Row(children: [
            Expanded(
              child: SettingsField(
                fieldKey: const ValueKey('hub-address-field'),
                controller: _addr,
                mono: true,
                onSubmitted: (v) => setHubAddress(sync, s, v.trim()),
              ),
            ),
            const SizedBox(width: 6),
            TextBtn('Apply', onTap: () => setHubAddress(sync, s, _addr.text.trim()), color: c.textSecondary),
          ]),
        ),
      ),
      if (sync.hubError != null)
        Padding(
          padding: const EdgeInsets.symmetric(vertical: 6),
          child: Text(sync.hubError!, key: const ValueKey('hub-error'), style: tsRaw(13, 18, 400, c.inlineCodeText)),
        ),
      if (bound != null)
        SettingsRow(
          label: 'Hub addresses',
          description: 'Serving on $bound. Devices can reach it at:',
          stack: true,
          trailing: Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
            for (final (url, kind) in _urls)
              Padding(
                padding: const EdgeInsets.only(bottom: 4),
                child: Row(children: [
                  Text(url, style: ts(const TypeSpec(13, 19, 400), c.textPrimary, mono: true)),
                  const SizedBox(width: 8),
                  Text(kind, style: tsRaw(12, 16, 400, c.textTertiary)),
                ]),
              ),
          ]),
        ),
      const SizedBox(height: 8),
      Row(children: [
        PrimaryButton(key: const ValueKey('add-device'), label: 'Add a device', onTap: () => showAddDevice(context)),
      ]),
    ]);
  }
}

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
      _hub.text = p.url;
      _token.text = p.token;
    }
    if (_error != null) setState(() => _error = null);
  }

  Future<void> _connect() async {
    final p = Pairing.parse(_hub.text);
    final url = p?.url ?? normalizeHubUrl(_hub.text);
    final token = p?.token ?? _token.text.trim();
    if (url.isEmpty || token.isEmpty) {
      setState(() => _error = 'Enter the hub address and the token, or paste a pairing code.');
      return;
    }
    setState(() => _busy = true);
    try {
      await connectToHub(context.read<SyncStore>(), AppSettings.read(context), url, token);
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
    if (!await confirmTrustCertificate(context, host: serverHostOf(url), fingerprint: fp)) return false;
    if (!mounted) return false;
    await trustCertificate(this.context.read<SyncStore>(), AppSettings.read(this.context), url, chain, fingerprint: fp);
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
    final sync = context.watch<SyncStore>();
    final s = AppSettings.of(context);
    if (sync.connected) {
      final (phase, message) = sync.phase;
      final ok = sync.status.lastOkMs;
      return Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
        SettingsRow(
          label: 'Connected to ${sync.hubUrl}',
          description: [
            phaseLabel(phase),
            if (ok != null) 'last synced ${relativeTime(ok.toInt()).toLowerCase()}',
          ].join(' · '),
          trailing: SyncIcon(phaseIcon(phase), size: 20, color: phase == SyncPhase.error ? c.inlineCodeText : c.iconSecondary),
        ),
        if (sync.untrustedCertificate)
          const Padding(
            padding: EdgeInsets.only(bottom: 8),
            child: SyncCertificateProblem(messageKey: ValueKey('connect-error')),
          )
        else if (message != null)
          Padding(
            padding: const EdgeInsets.only(bottom: 8),
            child: Text(message, key: const ValueKey('connect-error'), style: tsRaw(13, 18, 400, c.inlineCodeText)),
          ),
        const SizedBox(height: 4),
        Wrap(spacing: 8, runSpacing: 8, children: [
          OutlineButton(key: const ValueKey('sync-now-button'), label: 'Sync now', onTap: sync.syncNow),
          if (s.clientDeviceLabel == null) OutlineButton(
            key: const ValueKey('disconnect-button'),
            label: 'Disconnect',
            danger: true,
            onTap: () => disconnectFromHub(sync, s),
          ),
        ]),
      ]);
    }
    return Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
      Text(
        SyncStore.canHost
            ? 'Sync this vault with a hub on another computer instead.'
            : 'On the computer that runs the hub, open Settings › Sync › Add a device, then paste the pairing code here.',
        style: context.mobile ? tsRaw(15, 21, 400, c.textSecondary) : tsRaw(14, 20, 400, c.textSecondary),
      ),
      const SizedBox(height: 14),
      _label(context, 'Pairing code or hub address'),
      SettingsField(
        fieldKey: const ValueKey('connect-hub-field'),
        controller: _hub,
        placeholder: 'notes://pair?… or http://100.101.102.103:47100',
        onChanged: _hubChanged,
      ),
      const SizedBox(height: 10),
      _label(context, 'Token'),
      SettingsField(
        fieldKey: const ValueKey('connect-token-field'),
        controller: _token,
        mono: true,
        placeholder: 'Filled in by the pairing code',
        onSubmitted: (_) => _connect(),
      ),
      if (_error != null)
        Padding(
          padding: const EdgeInsets.only(top: 8),
          child: isUntrustedCertificateError(_error)
              ? _failedCertificate(context)
              : Text(_error!, style: tsRaw(13, 18, 400, c.inlineCodeText)),
        ),
      const SizedBox(height: 14),
      Row(children: [
        PrimaryButton(key: const ValueKey('connect-button'), label: _busy ? 'Connecting…' : 'Connect', onTap: _busy ? () {} : _connect),
      ]),
    ]);
  }

  Widget _label(BuildContext context, String text) => Padding(
        padding: const EdgeInsets.only(bottom: 6),
        child: Text(text, style: tsRaw(context.mobile ? 13 : 12, 16, 500, context.colors.textSecondary)),
      );
}

class AttachmentSettings extends StatelessWidget {
  const AttachmentSettings({super.key});

  @override
  Widget build(BuildContext context) {
    final sync = context.watch<SyncStore>();
    final s = AppSettings.of(context);
    final progress = sync.attachmentProgress;
    return Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
      SettingsRow(
        label: 'Download attachments in the background',
        description: 'Otherwise files and images download when a page shows them.',
        trailing: Toggle(
          key: const ValueKey('prefetch-toggle'),
          value: s.prefetchAttachments,
          onChanged: (v) => setPrefetch(sync, s, v),
        ),
      ),
      SettingsRow(
        label: 'Download all attachments',
        description: progress != null
            ? 'Downloading ${progress.$1} of ${progress.$2}…'
            : sync.connected
                ? 'Get every file now, to have them offline.'
                : 'Connect to a hub first.',
        trailing: OutlineButton(
          key: const ValueKey('download-attachments'),
          label: 'Download',
          onTap: sync.connected && progress == null ? () => sync.fetchAllAttachments() : null,
        ),
      ),
    ]);
  }
}

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

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final m = context.mobile;
    final pins = AppSettings.of(context).trustedCertificates;
    if (pins.isEmpty) return const SizedBox.shrink();
    return Column(key: const ValueKey('trusted-certificates'), crossAxisAlignment: CrossAxisAlignment.stretch, children: [
      const SizedBox(height: 28),
      const SettingsHeading('Trusted certificates'),
      Text('Hub certificates with an issuer this device doesn’t know, trusted on this device only.',
          style: m ? tsRaw(15, 21, 400, c.textSecondary) : tsRaw(14, 20, 400, c.textSecondary)),
      const SizedBox(height: 8),
      for (final (origin, pin) in pins.entries) ...[
        Padding(
          key: ValueKey('cert-pin-$origin'),
          padding: const EdgeInsets.symmetric(vertical: 4),
          child: Row(children: [
            Expanded(
              child: Hover(
                key: ValueKey('cert-pin-open-$origin'),
                onTap: () => _open(context, origin, pin),
                radius: Radii.md,
                padding: const EdgeInsets.symmetric(horizontal: 4, vertical: 6),
                child: Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
                  Text(serverHostOf(origin), style: tsRaw(m ? 16 : 14, 20, 500, c.textPrimary)),
                  const SizedBox(height: 2),
                  Text.rich(
                    TextSpan(children: [
                      TextSpan(
                          text: shortFingerprint(pin.sha256),
                          style: ts(const TypeSpec(12, 16, 400), c.textSecondary, mono: true)),
                      if (pin.notAfterMs > 0) TextSpan(text: ' · valid until ${formatCertDate(pin.notAfterMs)}'),
                    ]),
                    style: tsRaw(12, 16, 400, c.textSecondary),
                  ),
                ]),
              ),
            ),
            const SizedBox(width: 8),
            OutlineButton(
                key: ValueKey('cert-pin-revoke-$origin'), label: 'Revoke', onTap: () => _revoke(context, origin)),
          ]),
        ),
        HLine(color: c.borderSubtle),
      ],
    ]);
  }
}

// ---------------------------------------------------------------------------
// Devices

class DevicesBody extends StatefulWidget {
  const DevicesBody({super.key});

  @override
  State<DevicesBody> createState() => _DevicesBodyState();
}

class _DevicesBodyState extends State<DevicesBody> {
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
    final c = context.colors;
    final sync = context.watch<SyncStore>();
    final m = context.mobile;
    return Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
      const SettingsHeading('Devices', primary: true),
      Text('Devices paired with the hub on this computer. Unpairing revokes a device\'s token; it stops syncing.',
          style: m ? tsRaw(15, 21, 400, c.textSecondary) : tsRaw(14, 20, 400, c.textSecondary)),
      const SizedBox(height: 16),
      if (sync.devices.isEmpty)
        Padding(
          padding: const EdgeInsets.symmetric(vertical: 12),
          child: Text('No devices yet.', style: tsRaw(14, 20, 400, c.textTertiary)),
        ),
      for (final d in sync.devices) ...[
        Padding(
          key: ValueKey('device-${d.label}'),
          padding: const EdgeInsets.symmetric(vertical: 8),
          child: Row(children: [
            SyncIcon(SyncIco.device, size: 20, color: c.iconSecondary),
            const SizedBox(width: 12),
            Expanded(
              child: Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
                Text(d.label, style: tsRaw(m ? 16 : 14, 20, 500, c.textPrimary)),
                Text(
                  [
                    d.device == null ? 'Not connected yet' : 'Device ${d.device!.substring(0, d.device!.length.clamp(0, 8))}',
                    'paired ${relativeTime(d.createdMs.toInt()).toLowerCase()}',
                  ].join(' · '),
                  style: tsRaw(12, 16, 400, c.textSecondary),
                ),
              ]),
            ),
            OutlineButton(key: ValueKey('unpair-${d.label}'), label: 'Unpair', onTap: () => _unpair(d.label)),
          ]),
        ),
        HLine(color: c.borderSubtle),
      ],
      const SizedBox(height: 16),
      Row(children: [
        PrimaryButton(key: const ValueKey('devices-add'), label: 'Add a device', onTap: () => showAddDevice(context)),
      ]),
    ]);
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
      final code = Pairing(url, token).encode();
      children.addAll([
        Text('On the device, open $appName › Settings › Sync › Connect to a hub, and scan or paste this code. '
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
            child: Text('The hub is off. Turn it on in Sync so the device can connect.',
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
