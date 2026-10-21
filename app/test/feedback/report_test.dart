import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/feedback/report.dart';

const _deviceToken = 'DEV-SECRET-7f3a9c1e';
const _hubToken = 'HUB-SECRET-b2d4e6f8';

Diagnostics _diagnostics({
  String platform = 'linux',
  String themeSetting = 'system',
  bool effectiveDark = true,
  String? hubUrl,
  String? lastError,
  List<String> logs = const ['first log line', 'second log line'],
  List<String> errors = const ['Bad thing\n#0 main (file.dart:1)'],
}) => Diagnostics(
  appName: 'Scratch',
  appVersion: '0.1.0',
  buildNumber: '1',
  gitCommit: 'abc1234',
  platform: platform,
  osVersion: 'Linux 6.8',
  deviceModel: 'Test device',
  locale: 'en-US',
  screenSize: (390, 844),
  pixelRatio: 2.5,
  themeSetting: themeSetting,
  effectiveDark: effectiveDark,
  route: 'PageRoute',
  pageId: 'page-1',
  pageTitle: 'Inbox',
  signedIn: true,
  hubUrl: hubUrl,
  lastSync: DateTime(2026, 10, 16, 8, 30),
  lastError: lastError,
  scheduleRunner: false,
  pageCount: 12,
  logs: logs,
  errors: errors,
);

FeedbackReport _report({
  String text = 'Hello\n\nworld',
  String? screenshotId = 'attach-1',
  Diagnostics? diagnostics,
  List<String> secrets = const [],
}) => buildFeedbackReport(
  now: DateTime(2026, 10, 16, 9, 5),
  text: text,
  screenshotId: screenshotId,
  diagnostics: diagnostics ?? _diagnostics(),
  secrets: secrets,
);

void main() {
  group('title', () {
    final now = DateTime(2026, 10, 16, 9, 5);
    final cases = {
      'android': 'Feedback 2026-10-16 09:05 Android',
      'ios': 'Feedback 2026-10-16 09:05 iOS',
      'windows': 'Feedback 2026-10-16 09:05 Windows',
      'macos': 'Feedback 2026-10-16 09:05 macOS',
      'linux': 'Feedback 2026-10-16 09:05 Linux',
      'web': 'Feedback 2026-10-16 09:05 Web',
    };
    for (final entry in cases.entries) {
      test('${entry.key} is labelled ${entry.value.split(' ').last}', () {
        expect(reportTitle(now, entry.key), entry.value);
      });
    }

    test('uses local time and pads each field', () {
      expect(
        reportTitle(DateTime(2026, 1, 2, 3, 4), 'linux'),
        'Feedback 2026-01-02 03:04 Linux',
      );
    });

    test('is the title of the built report', () {
      expect(_report().title, 'Feedback 2026-10-16 09:05 Linux');
    });
  });

  group('body', () {
    test('is text paragraphs, then image, then code', () {
      final blocks = _report().blocks;
      expect(
        [for (final b in blocks) b.kind],
        ['paragraph', 'paragraph', 'image', 'code'],
      );
      expect(blocks[0].text, 'Hello');
      expect(blocks[1].text, 'world');
      expect(blocks[2].target, 'attach-1');
      expect(blocks[2].text, '');
      expect(blocks[3].target, 'text');
      expect(blocks[3].text, startsWith('app: Scratch\n'));
    });

    test('has no image when the screenshot was removed', () {
      final blocks = _report(screenshotId: null).blocks;
      expect(
        [for (final b in blocks) b.kind],
        ['paragraph', 'paragraph', 'code'],
      );
    });

    test('an empty text gives only the image and the code', () {
      final blocks = _report(text: '').blocks;
      expect([for (final b in blocks) b.kind], ['image', 'code']);
    });

    test('an empty text without a screenshot gives only the code', () {
      final blocks = _report(text: '', screenshotId: null).blocks;
      expect([for (final b in blocks) b.kind], ['code']);
    });

    test('blank lines in the text make no paragraph', () {
      final blocks = _report(
        text: '  \none\r\n\n   \ntwo',
        screenshotId: null,
      ).blocks;
      expect([for (final b in blocks.take(2)) b.text], ['one', 'two']);
      expect(blocks.last.kind, 'code');
      expect(blocks, hasLength(3));
    });
  });

  group('diagnostics', () {
    test('has every expected key and both sections', () {
      final text = formatDiagnostics(_diagnostics());
      for (final key in [
        'app: Scratch',
        'app version: 0.1.0',
        'build number: 1',
        'git commit: abc1234',
        'platform: Linux',
        'os version: ',
        'device model: ',
        'locale: en-US',
        'screen (logical): 390x844',
        'pixel ratio: 2.5',
        'theme: system (dark)',
        'route: PageRoute',
        'page id: page-1',
        'page title: Inbox',
        'signed in: yes',
        'hub url: none',
        'last sync: 2026-10-16T08:30:00.000',
        'last error: none',
        'schedule runner: off',
        'page count: 12',
        '--- errors ---',
        '--- log (last 2 lines) ---',
      ]) {
        expect(text, contains(key));
      }
      expect(
        text.indexOf('--- errors ---'),
        lessThan(text.indexOf('--- log (last 2 lines) ---')),
      );
    });

    test('unset values read as none or never', () {
      final text = formatDiagnostics(
        Diagnostics(
          appName: 'Scratch',
          appVersion: '0.1.0',
          buildNumber: '1',
          gitCommit: '',
          platform: 'android',
          osVersion: '14',
          deviceModel: 'x',
          locale: 'en',
          screenSize: null,
          pixelRatio: null,
          themeSetting: 'light',
          effectiveDark: false,
          route: '',
          pageId: null,
          pageTitle: null,
          signedIn: false,
          hubUrl: null,
          lastSync: null,
          lastError: null,
          scheduleRunner: true,
          pageCount: 0,
          logs: const [],
          errors: const [],
        ),
      );
      expect(text, contains('git commit: none'));
      expect(text, contains('screen (logical): unknown'));
      expect(text, contains('route: none'));
      expect(text, contains('last sync: never'));
      expect(text, contains('signed in: no'));
      expect(text, contains('schedule runner: on'));
      expect(text, contains('--- log (last 0 lines) ---\nnone'));
    });

    test('shows the theme setting and the brightness drawn', () {
      String themeLine(String setting, bool dark) => formatDiagnostics(
        _diagnostics(themeSetting: setting, effectiveDark: dark),
      ).split('\n').firstWhere((l) => l.startsWith('theme: '));

      expect(themeLine('system', true), 'theme: system (dark)');
      expect(themeLine('system', false), 'theme: system (light)');
      expect(themeLine('dark', true), 'theme: dark (dark)');
      expect(themeLine('light', false), 'theme: light (light)');
    });

    test('keeps only the newest log lines', () {
      final logs = [for (var i = 0; i < maxLogLines + 5; i++) 'line $i'];
      final text = formatDiagnostics(_diagnostics(logs: logs));
      expect(text, contains('--- log (last $maxLogLines lines) ---'));
      expect(text, isNot(contains('line 4\n')));
      expect(text, contains('line ${maxLogLines + 4}'));
    });
  });

  group('hub url', () {
    test('keeps scheme, host, port and path only', () {
      expect(
        reportHubUrl(
          'https://user:pw@hub.example.com:8443/api/sync?token=$_hubToken#top',
        ),
        'https://hub.example.com:8443/api/sync',
      );
    });

    test('writes no port when none is given', () {
      expect(
        reportHubUrl('http://192.168.0.5/sync'),
        'http://192.168.0.5/sync',
      );
    });

    test('reads unusable input as a fixed label', () {
      expect(reportHubUrl('not a url $_hubToken'), '(unreadable)');
      expect(reportHubUrl(''), 'none');
      expect(reportHubUrl(null), 'none');
    });
  });

  group('redaction', () {
    test('replaces Bearer tokens and token-like URL parameters', () {
      expect(
        redact('Authorization: Bearer abc.def', const []),
        'Authorization: Bearer [redacted]',
      );
      expect(
        redact(
          'GET /x?a=1&key=zzz&Token=t1&secret=s&password=p&b=2 HTTP',
          const [],
        ),
        'GET /x?a=1&key=[redacted]&Token=[redacted]&secret=[redacted]&password=[redacted]&b=2 HTTP',
      );
    });

    test('replaces every given secret, longest first', () {
      expect(
        redact('x abcdef abc y', const ['abc', 'abcdef']),
        'x [redacted] [redacted] y',
      );
    });

    test('ignores empty secrets', () {
      expect(redact('unchanged', const ['', '  ']), 'unchanged');
    });
  });

  group('secret safety', () {
    test('no device or hub token reaches the title or any block', () {
      final leaky = _diagnostics(
        hubUrl: 'https://hub.example.com/sync?token=$_hubToken',
        lastError: 'sync failed with $_hubToken',
        logs: ['device $_deviceToken connected', 'hub $_hubToken replied'],
        errors: ['Oops $_deviceToken\n#0 run (hub.dart:9 token=$_hubToken)'],
      );
      final report = _report(
        text:
            'my device $_deviceToken and Bearer $_hubToken\nhttps://a.example/?key=$_hubToken',
        diagnostics: leaky,
        secrets: const [_deviceToken, _hubToken],
      );

      final everything = [
        report.title,
        for (final b in report.blocks) ...[b.kind, b.text, b.target],
      ].join('\n');
      expect(everything, isNot(contains(_deviceToken)));
      expect(everything, isNot(contains(_hubToken)));
      expect(everything, contains(redacted));
    });

    test('the hub token is removed even without the secrets list', () {
      final report = _report(
        diagnostics: _diagnostics(
          hubUrl: 'https://hub.example.com/sync?token=$_hubToken',
          lastError: 'Bearer $_hubToken',
        ),
      );
      final everything = [for (final b in report.blocks) b.text].join('\n');
      expect(everything, isNot(contains(_hubToken)));
    });
  });
}
