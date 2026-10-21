// Certificates with an unknown issuer (self-signed or a private CA):
// - the error shown where sync errors show (indicator details, Sync
//   settings), with "View certificate" and a small "Trust certificate";
// - the confirmation that names the host and shows the fingerprint;
// - the "Certificate" view: the leaf's details, the chain, and one action
//   (trust it, or revoke the trust).
// Trust is pinned per server origin in the device settings (cert_trust.dart).

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';

import '../../core/models.dart';
import '../../state/app_state.dart';
import '../../state/sync_store.dart';
import '../../ui/ui.dart';
import '../settings/controls.dart';
import '../settings/settings_screen.dart';
import '../settings/settings_store.dart';
import 'cert_trust.dart';
import 'sync_control.dart';

/// Where a certificate stands for its server.
enum CertStatus { untrusted, changed, trusted }

CertDetails certDetailsOf(CertificateInfo i) => CertDetails(
      subject: i.subject,
      issuer: i.issuer,
      notBeforeMs: i.notBeforeMs.toInt(),
      notAfterMs: i.notAfterMs.toInt(),
      sha256: i.sha256,
      serial: i.serial,
      selfSigned: i.selfSigned,
    );

/// "Mar 3, 2027".
String formatCertDate(int ms) {
  const months = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'];
  final t = DateTime.fromMillisecondsSinceEpoch(ms);
  return '${months[t.month - 1]} ${t.day}, ${t.year}';
}

// ---------------------------------------------------------------------------
// The current client's untrusted certificate

/// The chain the hub presented (leaf first), or only the fingerprint the
/// error named when the chain is not known.
List<CertDetails> syncCertificateChain(SyncStore sync) {
  final chain = sync.untrustedChain;
  if (chain != null && chain.isNotEmpty) return [for (final c in chain) certDetailsOf(c)];
  final fp = sync.untrustedFingerprint;
  return fp == null ? const [] : [CertDetails(sha256: fp)];
}

/// Opens the view for the certificate the current hub presented.
Future<void> showSyncCertificate(BuildContext context) async {
  final sync = context.read<SyncStore>();
  final s = AppSettings.read(context);
  final url = sync.hubUrl;
  if (url == null) return;
  final fp = sync.untrustedFingerprint;
  final chain = syncCertificateChain(sync);
  if (chain.isEmpty) return;
  await showCertificate(
    context,
    host: serverHostOf(url),
    status: s.trustedCertificates.changed(url, fp) ? CertStatus.changed : CertStatus.untrusted,
    chain: chain,
    onTrust: (ctx) => trustSyncCertificate(ctx),
  );
}

/// Confirms, then trusts the certificate the current hub presented and
/// reconnects. True when trusted.
Future<bool> trustSyncCertificate(BuildContext context) async {
  final sync = context.read<SyncStore>();
  final s = AppSettings.read(context);
  final url = sync.hubUrl;
  final chain = syncCertificateChain(sync);
  if (url == null || chain.isEmpty) return false;
  final fp = sync.untrustedFingerprint ?? chain.first.sha256;
  if (!await confirmTrustCertificate(context, host: serverHostOf(url), fingerprint: fp)) return false;
  await trustCertificate(sync, s, url, chain, fingerprint: fp);
  return true;
}

/// The untrusted-certificate error of the current client, for the places
/// that show sync errors; [messageKey] goes on the first line.
class SyncCertificateProblem extends StatelessWidget {
  const SyncCertificateProblem({super.key, this.messageKey, this.onView, this.onTrust});
  final Key? messageKey;

  /// Default: open the view / confirm and trust.
  final VoidCallback? onView, onTrust;

  @override
  Widget build(BuildContext context) {
    final sync = context.watch<SyncStore>();
    final s = AppSettings.of(context);
    final url = sync.hubUrl ?? '';
    return CertificateProblem(
      messageKey: messageKey,
      host: serverHostOf(url),
      changed: s.trustedCertificates.changed(url, sync.untrustedFingerprint),
      onView: onView ?? () => showSyncCertificate(context),
      onTrust: onTrust ?? () => trustSyncCertificate(context),
    );
  }
}

// ---------------------------------------------------------------------------
// The error

/// "Can't verify the certificate for [host]." with what it means and the
/// two actions: "View certificate" leads, "Trust certificate" is small.
class CertificateProblem extends StatelessWidget {
  const CertificateProblem({
    super.key,
    required this.host,
    required this.changed,
    required this.onView,
    required this.onTrust,
    this.messageKey,
  });

  final String host;

  /// A pin exists for this server with another fingerprint.
  final bool changed;
  final VoidCallback onView, onTrust;
  final Key? messageKey;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final title = changed
        ? 'The certificate for $host changed since you trusted it.'
        : "Can't verify the certificate for $host.";
    final body = changed
        ? 'Trust the new one only if you changed it yourself.'
        : "Its issuer isn't known to this device, for example a self-signed or private CA certificate. "
            'Sync is paused; your notes are safe on this device.';
    return Column(
      key: const ValueKey('cert-problem'),
      crossAxisAlignment: CrossAxisAlignment.start,
      mainAxisSize: MainAxisSize.min,
      children: [
        Text(title, key: messageKey, style: tsRaw(13, 18, 600, c.inlineCodeText)),
        const SizedBox(height: 4),
        Text(body, style: tsRaw(13, 18, 400, c.textSecondary)),
        const SizedBox(height: 10),
        Wrap(spacing: 4, runSpacing: 6, crossAxisAlignment: WrapCrossAlignment.center, children: [
          OutlineButton(key: const ValueKey('cert-view'), label: 'View certificate', onTap: onView),
          IntrinsicWidth(child: TextBtn('Trust certificate', key: const ValueKey('cert-trust'), onTap: onTrust, color: c.textSecondary)),
        ]),
      ],
    );
  }
}

// ---------------------------------------------------------------------------
// Confirmations

/// "Trust this certificate?" naming [host] and showing [fingerprint]
/// (selectable). True when confirmed.
Future<bool> confirmTrustCertificate(BuildContext context, {required String host, required String fingerprint}) async {
  final r = await showModalPanel<bool>(
    context,
    (ctx) => _TrustConfirm(host: host, fingerprint: formatFingerprint(fingerprint), onDone: (v) => Navigator.of(ctx).pop(v)),
    width: 360,
    shrink: true,
  );
  return r ?? false;
}

class _TrustConfirm extends StatelessWidget {
  const _TrustConfirm({required this.host, required this.fingerprint, required this.onDone});
  final String host, fingerprint;
  final ValueChanged<bool> onDone;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final ty = context.type;
    return SingleChildScrollView(
      padding: const EdgeInsets.fromLTRB(20, 20, 20, 16),
      child: Column(
        key: const ValueKey('cert-trust-confirm'),
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Text('Trust this certificate?', textAlign: TextAlign.center, style: tsDisplay(const TypeSpec(19, 24, 640), c.textPrimary)),
          const SizedBox(height: 6),
          Text(
            'Sync will accept this certificate from $host. Trust it only if this SHA-256 fingerprint matches your hub’s.',
            textAlign: TextAlign.center,
            style: ts(ty.uiSmall, c.textSecondary, weight: 400),
          ),
          const SizedBox(height: 12),
          FingerprintBox(fingerprint, textKey: const ValueKey('cert-confirm-fingerprint'), copy: false),
          const SizedBox(height: 16),
          DialogButton(key: const ValueKey('confirm-ok'), label: 'Trust', color: c.accent, filled: true, onTap: () => onDone(true)),
          const SizedBox(height: 8),
          DialogButton(key: const ValueKey('confirm-cancel'), label: 'Cancel', onTap: () => onDone(false)),
        ],
      ),
    );
  }
}

/// "Stop trusting the certificate for [host]?" True when confirmed.
Future<bool> confirmRevokeCertificate(BuildContext context, String host) => showConfirm(
      context,
      title: 'Stop trusting the certificate for $host?',
      message: 'Sync with this hub stops until you trust it again.',
      confirmLabel: 'Revoke',
      danger: true,
    );

/// A fingerprint in monospace that can be selected, with a "Copy" button.
class FingerprintBox extends StatefulWidget {
  const FingerprintBox(this.text, {super.key, this.textKey, this.copy = true});
  final String text;
  final Key? textKey;
  final bool copy;

  @override
  State<FingerprintBox> createState() => _FingerprintBoxState();
}

class _FingerprintBoxState extends State<FingerprintBox> {
  final _focus = FocusNode();
  bool _copied = false;

  @override
  void dispose() {
    _focus.dispose();
    super.dispose();
  }

  Future<void> _copy() async {
    await Clipboard.setData(ClipboardData(text: widget.text));
    if (!mounted) return;
    setState(() => _copied = true);
    await Future<void>.delayed(const Duration(seconds: 2));
    if (mounted) setState(() => _copied = false);
  }

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    return Container(
      padding: EdgeInsets.fromLTRB(10, 6, widget.copy ? 4 : 10, 6),
      decoration: BoxDecoration(color: c.bgSecondary, borderRadius: Radii.field, border: Border.all(color: c.borderSubtle)),
      child: Row(children: [
        Expanded(
          child: SelectableRegion(
            focusNode: _focus,
            selectionControls: noteSelectionControls,
            // Eight pairs per line, so lines break between pairs, never inside one.
            child: Text(widget.text.replaceAllMapped(RegExp(r'((?:[0-9A-F]{2}:){8})'), (m) => '${m[1]}\n'), key: widget.textKey, style: ts(const TypeSpec(13, 19, 400), c.textPrimary, mono: true)),
          ),
        ),
        if (widget.copy) ...[
          const SizedBox(width: 8),
          TextBtn(_copied ? 'Copied' : 'Copy', key: const ValueKey('cert-copy'), onTap: _copy, color: c.textSecondary),
        ],
      ]),
    );
  }
}

// ---------------------------------------------------------------------------
// The view

/// Opens the certificate view: a page on mobile, a panel on desktop (as
/// "Add a device" does). [onTrust] / [onRevoke] run on the view's context
/// and return true when done; the view then closes.
Future<void> showCertificate(
  BuildContext context, {
  required String host,
  required CertStatus status,
  required List<CertDetails> chain,
  int? trustedAtMs,
  Future<bool> Function(BuildContext context)? onTrust,
  Future<bool> Function(BuildContext context)? onRevoke,
}) {
  final view = CertificateView(
    host: host,
    status: status,
    chain: chain,
    trustedAtMs: trustedAtMs,
    onTrust: onTrust,
    onRevoke: onRevoke,
  );
  if (context.mobile) return pushSettingsPage(context, 'Certificate', view);
  return showPanel(
    context,
    (_) => SingleChildScrollView(padding: const EdgeInsets.fromLTRB(40, 32, 40, 32), child: view),
    width: 560,
    height: 760,
    shrink: true,
  );
}

class CertificateView extends StatefulWidget {
  const CertificateView({
    super.key,
    required this.host,
    required this.status,
    required this.chain,
    this.trustedAtMs,
    this.onTrust,
    this.onRevoke,
    this.nowMs,
  });

  final String host;
  final CertStatus status;

  /// Leaf first.
  final List<CertDetails> chain;
  final int? trustedAtMs;
  final Future<bool> Function(BuildContext context)? onTrust, onRevoke;

  /// For "Expired" (default: now).
  final int? nowMs;

  @override
  State<CertificateView> createState() => _CertificateViewState();
}

class _CertificateViewState extends State<CertificateView> {
  final _open = <int>{};
  bool _busy = false;

  Future<void> _run(Future<bool> Function(BuildContext) f) async {
    setState(() => _busy = true);
    var done = false;
    try {
      done = await f(context);
    } finally {
      if (mounted) setState(() => _busy = false);
    }
    if (done && mounted) await Navigator.of(context).maybePop();
  }

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final m = context.mobile;
    final w = widget;
    final now = w.nowMs ?? clockNow().millisecondsSinceEpoch;
    final status = switch (w.status) {
      CertStatus.untrusted => 'Not trusted — unknown issuer',
      CertStatus.changed => 'Changed since you trusted it',
      CertStatus.trusted => w.trustedAtMs != null && w.trustedAtMs! > 0
          ? 'Trusted since ${formatCertDate(w.trustedAtMs!)}'
          : 'Trusted',
    };
    final leaf = w.chain.isEmpty ? null : w.chain.first;
    final trusted = w.status == CertStatus.trusted;
    return Column(
      key: const ValueKey('cert-view-body'),
      crossAxisAlignment: CrossAxisAlignment.stretch,
      mainAxisSize: MainAxisSize.min,
      children: [
        if (!m) ...[
          Text('Certificate', style: tsRaw(20, 26, 600, c.textPrimary)),
          const SizedBox(height: 16),
        ],
        Text(w.host, key: const ValueKey('cert-host'), style: tsRaw(m ? 17 : 16, 22, 600, c.textPrimary)),
        const SizedBox(height: 2),
        Text(status,
            key: const ValueKey('cert-status'),
            style: tsRaw(m ? 15 : 14, 20, 400, trusted ? c.textSecondary : c.inlineCodeText)),
        const SizedBox(height: 16),
        if (leaf != null) CertFields(leaf, nowMs: now, keyPrefix: 'cert-leaf'),
        if (w.chain.isNotEmpty) ...[
          const SizedBox(height: 20),
          const SettingsHeading('Chain'),
          for (var i = 0; i < w.chain.length; i++) ...[
            _ChainRow(
              index: i,
              cert: w.chain[i],
              open: _open.contains(i),
              onTap: () => setState(() => _open.contains(i) ? _open.remove(i) : _open.add(i)),
            ),
            if (_open.contains(i))
              Padding(
                padding: const EdgeInsets.fromLTRB(26, 4, 0, 12),
                child: CertFields(w.chain[i], nowMs: now, keyPrefix: 'cert-chain-$i'),
              ),
            HLine(color: c.borderSubtle),
          ],
        ],
        const SizedBox(height: 20),
        Row(children: [
          if (trusted && w.onRevoke != null)
            OutlineButton(
              key: const ValueKey('cert-revoke'),
              label: 'Revoke trust',
              danger: true,
              onTap: _busy ? null : () => _run(w.onRevoke!),
            )
          else if (!trusted && w.onTrust != null)
            OutlineButton(
              key: const ValueKey('cert-view-trust'),
              label: 'Trust certificate',
              onTap: _busy ? null : () => _run(w.onTrust!),
            ),
        ]),
      ],
    );
  }
}

class _ChainRow extends StatelessWidget {
  const _ChainRow({required this.index, required this.cert, required this.open, required this.onTap});
  final int index;
  final CertDetails cert;
  final bool open;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final m = context.mobile;
    final name = cert.subject.isEmpty ? 'Certificate ${index + 1}' : cert.subject;
    return Semantics(
      button: true,
      expanded: open,
      child: Hover(
        key: ValueKey('cert-chain-row-$index'),
        onTap: onTap,
        radius: Radii.md,
        padding: const EdgeInsets.symmetric(horizontal: 4),
        child: ConstrainedBox(
          constraints: BoxConstraints(minHeight: m ? 44 : 36),
          child: Padding(
            padding: const EdgeInsets.symmetric(vertical: 8),
            child: Row(crossAxisAlignment: CrossAxisAlignment.start, children: [
              Padding(
                padding: const EdgeInsets.only(top: 2),
                child: Icon20(open ? Ico.chevronDown : Ico.chevronRight, size: 16, color: c.iconSecondary),
              ),
              const SizedBox(width: 6),
              Expanded(child: Text(name, style: tsRaw(m ? 15 : 14, 20, 500, c.textPrimary))),
            ]),
          ),
        ),
      ),
    );
  }
}

/// A certificate's details as label/value rows: side by side when wide,
/// stacked when narrow. Empty values are left out.
class CertFields extends StatelessWidget {
  const CertFields(this.cert, {super.key, required this.nowMs, this.keyPrefix = 'cert'});
  final CertDetails cert;
  final int nowMs;
  final String keyPrefix;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final m = context.mobile;
    final value = tsRaw(m ? 15 : 14, 20, 400, c.textPrimary);
    final mono = ts(const TypeSpec(13, 19, 400), c.textPrimary, mono: true);
    final expired = cert.expiredAt(nowMs);
    final rows = <(String, String, Widget)>[
      if (cert.subject.isNotEmpty) ('Subject', 'subject', Text(cert.subject, style: value)),
      if (cert.issuer.isNotEmpty) ('Issuer', 'issuer', Text(cert.issuer, style: value)),
      if (cert.notBeforeMs > 0) ('Valid from', 'from', Text(formatCertDate(cert.notBeforeMs), style: value)),
      if (cert.notAfterMs > 0)
        (
          'Valid until',
          'until',
          Text(
            expired ? '${formatCertDate(cert.notAfterMs)} · Expired' : formatCertDate(cert.notAfterMs),
            style: expired ? tsRaw(m ? 15 : 14, 20, 600, c.inlineCodeText) : value,
          ),
        ),
      ('SHA-256 fingerprint', 'sha256', FingerprintBox(formatFingerprint(cert.sha256), textKey: ValueKey('$keyPrefix-sha256-text'))),
      if (cert.serial.isNotEmpty) ('Serial', 'serial', Text(cert.serial, style: mono)),
      if (cert.selfSigned) ('Self-signed', 'self-signed', Text('Yes', style: value)),
    ];
    return LayoutBuilder(builder: (context, box) {
      final wide = box.maxWidth >= 440;
      return Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
        for (final (label, id, v) in rows)
          Padding(
            key: ValueKey('$keyPrefix-$id'),
            padding: const EdgeInsets.symmetric(vertical: 6),
            child: wide
                ? Row(crossAxisAlignment: CrossAxisAlignment.start, children: [
                    SizedBox(
                      width: 150,
                      child: Padding(
                        padding: const EdgeInsets.only(top: 2),
                        child: Text(label, style: tsRaw(13, 18, 500, c.textSecondary)),
                      ),
                    ),
                    Expanded(child: v),
                  ])
                : Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
                    Text(label, style: tsRaw(m ? 13 : 12, 16, 500, c.textSecondary)),
                    const SizedBox(height: 4),
                    v,
                  ]),
          ),
      ]);
    });
  }
}
