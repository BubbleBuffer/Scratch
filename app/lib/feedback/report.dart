// The feedback report, as plain values: the page title and the blocks in the
// order they go on the page (text paragraphs, the screenshot, the diagnostics
// code block). Nothing here imports the Rust bridge or app state, so it runs
// in unit tests. The caller turns [ReportBlock]s into insert ops.
//
// Secrets never leave this file: the caller passes every secret string, and
// Bearer tokens and token-like URL parameters are always redacted.

/// The text that replaces anything secret.
const redacted = '[redacted]';

/// Log lines kept in the report (the newest ones).
const maxLogLines = 200;

/// One block of the report, before it becomes an insert op.
class ReportBlock {
  const ReportBlock({required this.kind, this.text = '', this.target = ''});

  /// A block kind name as the core spells it: `paragraph`, `image` or `code`.
  final String kind;
  final String text;

  /// The attachment id of an `image`, or the fence info of a `code` block.
  final String target;
}

/// What the report says about the app and the device. The collector fills it.
class Diagnostics {
  const Diagnostics({
    required this.appName,
    required this.appVersion,
    required this.buildNumber,
    required this.gitCommit,
    required this.platform,
    required this.osVersion,
    required this.deviceModel,
    required this.locale,
    required this.screenSize,
    required this.pixelRatio,
    required this.themeSetting,
    required this.effectiveDark,
    required this.route,
    required this.pageId,
    required this.pageTitle,
    required this.signedIn,
    required this.hubUrl,
    required this.lastSync,
    required this.lastError,
    required this.scheduleRunner,
    required this.pageCount,
    required this.logs,
    required this.errors,
  });

  final String appName;
  final String appVersion;
  final String buildNumber;

  /// Empty when the build did not set GIT_COMMIT.
  final String gitCommit;

  /// The operating system name, as `Platform.operatingSystem` spells it.
  final String platform;
  final String osVersion;
  final String deviceModel;
  final String locale;

  /// Logical pixels (width, height); null when there is no view.
  final (double, double)? screenSize;
  final double? pixelRatio;

  /// The theme setting as the user chose it: `system`, `light` or `dark`.
  final String themeSetting;

  /// Whether the app is drawn dark right now, from the setting or the system.
  final bool effectiveDark;

  /// The current route's name; empty when there is none.
  final String route;
  final String? pageId;

  /// Only the title: never the page's contents.
  final String? pageTitle;
  final bool signedIn;
  final String? hubUrl;
  final DateTime? lastSync;
  final String? lastError;
  final bool scheduleRunner;
  final int pageCount;

  /// Log lines, oldest first.
  final List<String> logs;

  /// Error messages, each with its stack trace.
  final List<String> errors;
}

/// The report page's title and blocks, in page order.
class FeedbackReport {
  const FeedbackReport({required this.title, required this.blocks});

  final String title;
  final List<ReportBlock> blocks;
}

/// Builds the report: one paragraph per non-blank line of [text], then the
/// screenshot (when [screenshotId] is set), then the diagnostics code block.
/// [secrets] and every token-like string are redacted from all of it.
FeedbackReport buildFeedbackReport({
  required DateTime now,
  required String text,
  required String? screenshotId,
  required Diagnostics diagnostics,
  List<String> secrets = const [],
}) {
  String clean(String s) => redact(s, secrets);
  return FeedbackReport(
    title: clean(reportTitle(now, diagnostics.platform)),
    blocks: [
      for (final line in text.split('\n'))
        if (line.trim().isNotEmpty)
          ReportBlock(kind: 'paragraph', text: clean(line.trimRight())),
      if (screenshotId != null && screenshotId.isNotEmpty)
        ReportBlock(kind: 'image', target: clean(screenshotId)),
      ReportBlock(
        kind: 'code',
        text: clean(formatDiagnostics(diagnostics)),
        target: 'text',
      ),
    ],
  );
}

/// `Feedback yyyy-MM-dd HH:mm <platform>` in local time.
String reportTitle(DateTime now, String operatingSystem) {
  final local = now.toLocal();
  final stamp =
      '${local.year.toString().padLeft(4, '0')}-${_two(local.month)}-${_two(local.day)} '
      '${_two(local.hour)}:${_two(local.minute)}';
  return 'Feedback $stamp ${platformLabel(operatingSystem)}';
}

/// The name shown to people for a `Platform.operatingSystem` value.
String platformLabel(String operatingSystem) => switch (operatingSystem) {
  'android' => 'Android',
  'ios' => 'iOS',
  'windows' => 'Windows',
  'macos' => 'macOS',
  'linux' => 'Linux',
  'web' => 'Web',
  _ => 'Unknown',
};

/// The diagnostics as `key: value` lines, then the errors and the log tail.
String formatDiagnostics(Diagnostics d) {
  final logs = d.logs.length > maxLogLines
      ? d.logs.sublist(d.logs.length - maxLogLines)
      : d.logs;
  return [
    'app: ${d.appName}',
    'app version: ${d.appVersion}',
    'build number: ${d.buildNumber}',
    'git commit: ${d.gitCommit.isEmpty ? 'none' : d.gitCommit}',
    'platform: ${platformLabel(d.platform)}',
    'os version: ${d.osVersion}',
    'device model: ${d.deviceModel}',
    'locale: ${d.locale}',
    'screen (logical): ${_size(d.screenSize)}',
    'pixel ratio: ${d.pixelRatio ?? 'unknown'}',
    'theme: ${d.themeSetting} (${d.effectiveDark ? 'dark' : 'light'})',
    'route: ${d.route.isEmpty ? 'none' : d.route}',
    'page id: ${d.pageId ?? 'none'}',
    'page title: ${d.pageTitle ?? 'none'}',
    'signed in: ${d.signedIn ? 'yes' : 'no'}',
    'hub url: ${reportHubUrl(d.hubUrl)}',
    'last sync: ${d.lastSync?.toIso8601String() ?? 'never'}',
    'last error: ${d.lastError ?? 'none'}',
    'schedule runner: ${d.scheduleRunner ? 'on' : 'off'}',
    'page count: ${d.pageCount}',
    '',
    '--- errors ---',
    if (d.errors.isEmpty) 'none' else ...d.errors,
    '',
    '--- log (last ${logs.length} lines) ---',
    if (logs.isEmpty) 'none' else ...logs,
  ].join('\n');
}

/// The hub URL as scheme, host, port and path. User info, query and fragment
/// are dropped; an unreadable URL becomes a fixed label.
String reportHubUrl(String? url) {
  final trimmed = url?.trim() ?? '';
  if (trimmed.isEmpty) return 'none';
  final uri = Uri.tryParse(trimmed);
  if (uri == null || !uri.hasScheme || !uri.hasAuthority) return '(unreadable)';
  final host = uri.host.contains(':') ? '[${uri.host}]' : uri.host;
  final port = uri.hasPort ? ':${uri.port}' : '';
  return '${uri.scheme}://$host$port${uri.path}';
}

/// Replaces every entry of [secrets], every `Bearer` token and every URL
/// parameter named token, key, secret or password in [text].
String redact(String text, Iterable<String> secrets) {
  var out = text.replaceAllMapped(_urlParam, (m) => '${m[1]}$redacted');
  out = out.replaceAll(_bearer, 'Bearer $redacted');
  // Longest first, so a secret that contains a shorter one is removed whole.
  final literal = [
    for (final s in secrets)
      if (s.trim().isNotEmpty) s.trim(),
  ]..sort((a, b) => b.length.compareTo(a.length));
  for (final s in literal) {
    out = out.replaceAll(s, redacted);
  }
  return out;
}

final _urlParam = RegExp(
  r'([?&](?:token|key|secret|password)=)[^&\s#"<>]*',
  caseSensitive: false,
);
final _bearer = RegExp(r'Bearer\s+[^\s"<>]+', caseSensitive: false);

String _two(int n) => n.toString().padLeft(2, '0');

String _size((double, double)? size) =>
    size == null ? 'unknown' : '${size.$1.round()}x${size.$2.round()}';
