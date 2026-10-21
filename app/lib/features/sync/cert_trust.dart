// Certificates trusted on this device for hubs whose certificate has an
// unknown issuer (self-signed or a private CA). One pin per server origin
// (`scheme://host:port`), kept in the device's settings (never synced).
// Plain Dart, so the rules are testable without widgets:
// - origins are lowercased with the default port filled in;
// - trusting replaces the pin for that origin;
// - a pin whose fingerprint differs from the one the hub presents means the
//   certificate changed since it was trusted.

/// The prefix of a sync error for an unknown-issuer certificate that is not
/// pinned; the SHA-256 fingerprint follows.
const untrustedCertificatePrefix = 'untrusted certificate';

final _untrusted = RegExp(r'untrusted certificate:\s*([0-9A-Fa-f]{2}(?::?[0-9A-Fa-f]{2})*)');

/// Whether [message] is an untrusted-certificate sync error.
bool isUntrustedCertificateError(String? message) =>
    message != null && message.contains(untrustedCertificatePrefix);

/// The fingerprint in an untrusted-certificate error, uppercase with `:`.
String? untrustedFingerprintOf(String? message) {
  if (message == null) return null;
  final m = _untrusted.firstMatch(message);
  return m == null ? null : formatFingerprint(m.group(1)!);
}

/// Uppercase hex without separators, for comparing fingerprints.
String normalizeFingerprint(String fp) => fp.replaceAll(RegExp(r'[^0-9A-Fa-f]'), '').toUpperCase();

/// Uppercase hex pairs separated by `:`.
String formatFingerprint(String fp) {
  final h = normalizeFingerprint(fp);
  final b = StringBuffer();
  for (var i = 0; i < h.length; i += 2) {
    if (i > 0) b.write(':');
    b.write(h.substring(i, i + 2 > h.length ? h.length : i + 2));
  }
  return b.toString();
}

/// The first [pairs] pairs of a fingerprint and an ellipsis.
String shortFingerprint(String fp, {int pairs = 6}) {
  final parts = formatFingerprint(fp).split(':');
  return parts.length <= pairs ? parts.join(':') : '${parts.take(pairs).join(':')}…';
}

/// `scheme://host:port` of [url], lowercased, with the default port filled
/// in (80 for http, 443 for https). A URL without a scheme is taken as
/// http, as hub addresses are. Null if it has no host.
String? originOf(String url) {
  var u = url.trim();
  if (u.isEmpty) return null;
  if (!u.contains('://')) u = 'http://$u';
  final uri = Uri.tryParse(u);
  if (uri == null || uri.host.isEmpty) return null;
  final scheme = uri.scheme.toLowerCase();
  final host = uri.host.toLowerCase();
  final port = uri.hasPort ? uri.port : (scheme == 'https' ? 443 : scheme == 'http' ? 80 : 0);
  final h = host.contains(':') ? '[$host]' : host;
  return '$scheme://$h:$port';
}

/// The host of [url] for messages, with the port when it is not the
/// scheme's default.
String serverHostOf(String url) {
  final o = originOf(url);
  if (o == null) return url;
  final uri = Uri.parse(o);
  final def = uri.scheme == 'https' ? 443 : 80;
  final h = uri.host.contains(':') ? '[${uri.host}]' : uri.host;
  return uri.port == def ? h : '$h:${uri.port}';
}

/// One certificate's details, as shown on the certificate view (the
/// bridge's CertificateInfo, kept in the settings for later viewing).
class CertDetails {
  const CertDetails({
    this.subject = '',
    this.issuer = '',
    this.notBeforeMs = 0,
    this.notAfterMs = 0,
    required this.sha256,
    this.serial = '',
    this.selfSigned = false,
  });

  final String subject, issuer;
  final int notBeforeMs, notAfterMs;
  final String sha256;
  final String serial;
  final bool selfSigned;

  /// The bridge marks a certificate it could not parse this way.
  bool get unparsable => subject.startsWith('unparsable certificate');

  bool expiredAt(int nowMs) => notAfterMs > 0 && notAfterMs < nowMs;

  Map<String, dynamic> toJson() => {
        'subject': subject,
        'issuer': issuer,
        'notBeforeMs': notBeforeMs,
        'notAfterMs': notAfterMs,
        'sha256': sha256,
        'serial': serial,
        'selfSigned': selfSigned,
      };

  static CertDetails? fromJson(Object? j) {
    if (j is! Map) return null;
    final sha = j['sha256'];
    if (sha is! String || sha.isEmpty) return null;
    int n(Object? v) => v is num ? v.toInt() : 0;
    String s(Object? v) => v is String ? v : '';
    return CertDetails(
      subject: s(j['subject']),
      issuer: s(j['issuer']),
      notBeforeMs: n(j['notBeforeMs']),
      notAfterMs: n(j['notAfterMs']),
      sha256: sha,
      serial: s(j['serial']),
      selfSigned: j['selfSigned'] == true,
    );
  }

  @override
  bool operator ==(Object other) =>
      other is CertDetails &&
      other.subject == subject &&
      other.issuer == issuer &&
      other.notBeforeMs == notBeforeMs &&
      other.notAfterMs == notAfterMs &&
      other.sha256 == sha256 &&
      other.serial == serial &&
      other.selfSigned == selfSigned;

  @override
  int get hashCode => Object.hash(subject, issuer, notBeforeMs, notAfterMs, sha256, serial, selfSigned);
}

/// The trusted certificate of one origin: the leaf's fingerprint and
/// details, when it was trusted, and the chain (leaf first) for viewing.
class PinnedCertificate {
  const PinnedCertificate({
    required this.sha256,
    this.subject = '',
    this.issuer = '',
    this.notBeforeMs = 0,
    this.notAfterMs = 0,
    required this.trustedAtMs,
    this.chain = const [],
  });

  /// A pin for [chain] (leaf first); [fingerprint] wins over the leaf's
  /// when given (the one the error named).
  factory PinnedCertificate.of(List<CertDetails> chain, {String? fingerprint, required int trustedAtMs}) {
    final leaf = chain.isEmpty ? null : chain.first;
    return PinnedCertificate(
      sha256: formatFingerprint(fingerprint ?? leaf?.sha256 ?? ''),
      subject: leaf?.subject ?? '',
      issuer: leaf?.issuer ?? '',
      notBeforeMs: leaf?.notBeforeMs ?? 0,
      notAfterMs: leaf?.notAfterMs ?? 0,
      trustedAtMs: trustedAtMs,
      chain: chain,
    );
  }

  final String sha256, subject, issuer;
  final int notBeforeMs, notAfterMs, trustedAtMs;
  final List<CertDetails> chain;

  /// The chain to show: the stored one, or the leaf from the pin's own
  /// fields when none was stored.
  List<CertDetails> get viewChain => chain.isNotEmpty
      ? chain
      : [CertDetails(subject: subject, issuer: issuer, notBeforeMs: notBeforeMs, notAfterMs: notAfterMs, sha256: sha256)];

  bool matches(String fingerprint) => normalizeFingerprint(fingerprint) == normalizeFingerprint(sha256);

  Map<String, dynamic> toJson() => {
        'sha256': sha256,
        'subject': subject,
        'issuer': issuer,
        'notBeforeMs': notBeforeMs,
        'notAfterMs': notAfterMs,
        'trustedAtMs': trustedAtMs,
        'chain': [for (final c in chain) c.toJson()],
      };

  static PinnedCertificate? fromJson(Object? j) {
    if (j is! Map) return null;
    final sha = j['sha256'];
    if (sha is! String || normalizeFingerprint(sha).isEmpty) return null;
    int n(Object? v) => v is num ? v.toInt() : 0;
    String s(Object? v) => v is String ? v : '';
    final chain = j['chain'];
    return PinnedCertificate(
      sha256: sha,
      subject: s(j['subject']),
      issuer: s(j['issuer']),
      notBeforeMs: n(j['notBeforeMs']),
      notAfterMs: n(j['notAfterMs']),
      trustedAtMs: n(j['trustedAtMs']),
      chain: chain is List ? [for (final c in chain) ?CertDetails.fromJson(c)] : const [],
    );
  }
}

/// The pins of this device, keyed by origin.
class TrustedCertificates {
  TrustedCertificates([Map<String, PinnedCertificate>? pins]) : _pins = {...?pins};

  final Map<String, PinnedCertificate> _pins;

  bool get isEmpty => _pins.isEmpty;
  bool get isNotEmpty => _pins.isNotEmpty;

  /// Origin and pin, sorted by origin.
  List<(String, PinnedCertificate)> get entries =>
      [for (final k in _pins.keys.toList()..sort()) (k, _pins[k]!)];

  /// The pin for [url]'s origin.
  PinnedCertificate? lookup(String url) {
    final o = originOf(url);
    return o == null ? null : _pins[o];
  }

  /// The fingerprints to accept for [url] (empty without a pin).
  List<String> fingerprintsFor(String url) {
    final p = lookup(url);
    return p == null ? const [] : [p.sha256];
  }

  /// Trusts [pin] for [url]'s origin, replacing an earlier pin. Returns the
  /// origin (null for a URL without a host, which is not stored).
  String? add(String url, PinnedCertificate pin) {
    final o = originOf(url);
    if (o == null) return null;
    _pins[o] = pin;
    return o;
  }

  /// Removes the pin for [url]'s origin; true if there was one.
  bool revoke(String url) {
    final o = originOf(url);
    return o != null && _pins.remove(o) != null;
  }

  /// A pin exists for [url]'s origin, and the hub now presents a
  /// certificate with another fingerprint.
  bool changed(String url, String? fingerprint) {
    final p = lookup(url);
    return p != null && fingerprint != null && !p.matches(fingerprint);
  }

  Map<String, dynamic> toJson() => {for (final (o, p) in entries) o: p.toJson()};

  /// Tolerant: a missing or malformed value gives no pins, and malformed
  /// entries are skipped.
  static TrustedCertificates fromJson(Object? j) {
    final out = TrustedCertificates();
    if (j is! Map) return out;
    for (final e in j.entries) {
      final o = e.key is String ? originOf(e.key as String) : null;
      final p = PinnedCertificate.fromJson(e.value);
      if (o != null && p != null) out._pins[o] = p;
    }
    return out;
  }
}
