import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/features/settings/settings_kit.dart';
import 'package:notes_app/features/settings/settings_store.dart';
import 'package:notes_app/features/updates/update_controller.dart';
import 'package:notes_app/src/rust/api/update.dart';

import 'harness.dart';

Finder key(String name) => find.byKey(ValueKey(name));

const _release = UpdateRelease(
  tag: 'v0.1.0-prealpha.12',
  name: 'v0.1.0-prealpha.12',
  publishedAt: '2026-10-27T10:00:00Z',
  notesUrl: 'https://github.com/BubbleBuffer/Scratch/releases/tag/v0.1.0-prealpha.12',
  nightly: false,
  installerName: 'Scratch-Setup-0.1.0-prealpha.12.exe',
  installerUrl: 'https://github.com/BubbleBuffer/Scratch/releases/download/v0.1.0-prealpha.12/Scratch-Setup-0.1.0-prealpha.12.exe',
  installerSize: 100,
  sumsUrl: 'https://github.com/BubbleBuffer/Scratch/releases/download/v0.1.0-prealpha.12/SHA256SUMS.txt',
);

class FakeBackend implements UpdateBackend {
  UpdateCheck next = const UpdateCheck(status: UpdateCheckStatus.upToDate);
  Object? downloadError;
  final calls = <String>[];

  @override
  Future<UpdateCheck> check({required String currentTag, required bool includeNightly, required String updatesDir}) async {
    calls.add('check nightly=$includeNightly');
    return next;
  }

  @override
  Future<DownloadedUpdate> download(UpdateRelease release, {required String currentTag, required String updatesDir}) async {
    calls.add('download ${release.tag}');
    if (downloadError != null) throw downloadError!;
    return DownloadedUpdate(path: '$updatesDir/${release.installerName}', sha256: 'ab' * 32);
  }

  @override
  UpdateProgress progress() => const UpdateProgress(received: 50, total: 100, active: true);

  @override
  Future<void> install({required String path, required String sha256}) async => calls.add('install $path');

  @override
  Future<void> exitApp() async => calls.add('exit');
}

String statusText(WidgetTester tester) =>
    tester.widgetList<SRow>(find.byType(SRow)).firstWhere((r) => r.label == 'Status').desc!;

void main() {
  setUpAll(initCore);

  Future<(FakeBackend, UpdateController, TestVault)> openUpdates(
    WidgetTester tester, {
    String tag = 'v0.1.0-prealpha.11',
    bool autoDownload = false,
  }) async {
    final fake = FakeBackend();
    final settings = AppSettings()..updatesAutoDownload = autoDownload;
    final u = UpdateController(
      currentTag: tag,
      updatesDir: 'updates',
      settings: settings,
      backend: fake,
      now: () => DateTime(2026, 10, 27, 12, 5),
    );
    UpdateController.current = u;
    final tv = await pumpApp(tester, mobile: false);
    await tapAndSettle(tester, key('sidebar-settings'));
    await tapAndSettle(tester, key('settings-nav-about'));
    return (fake, u, tv);
  }

  tearDown(() {
    UpdateController.current?.dispose();
    UpdateController.current = null;
  });

  test('update settings default to stable channel with auto download', () {
    final s = AppSettings();
    expect(s.updatesIncludeNightly, isFalse);
    expect(s.updatesAutoDownload, isTrue);
    expect(s.toJson()['updatesIncludeNightly'], isFalse);
    expect(s.toJson()['updatesAutoDownload'], isTrue);
  });

  appTest('updates section: up to date, available, download, ready, install', (tester) async {
    final (fake, u, _) = await openUpdates(tester);
    expect(find.text('Scratch 0.1.0-prealpha.11 · Pre-release'), findsOneWidget);
    expect(statusText(tester), 'Not checked yet.');
    expect(key('update-ready-row'), findsNothing);

    await tapAndSettle(tester, key('updates-check-now'));
    expect(statusText(tester), 'Up to date. Checked today at 12:05.');
    expect(fake.calls, ['check nightly=false']);

    fake.next = const UpdateCheck(status: UpdateCheckStatus.available, release: _release);
    await tapAndSettle(tester, key('updates-check-now'));
    expect(statusText(tester), 'Version 0.1.0-prealpha.12 is available.');
    expect(key('updates-download'), findsOneWidget);
    expect(fake.calls.where((c) => c.startsWith('download')), isEmpty, reason: 'auto download is off');

    await tester.ensureVisible(key('updates-download'));
    await tapAndSettle(tester, key('updates-download'));
    expect(u.phase, UpdatePhase.ready);
    expect(statusText(tester), 'Version 0.1.0-prealpha.12 is ready to install.');
    expect(key('updates-install'), findsOneWidget);
    expect(key('update-ready-row'), findsOneWidget);

    await tester.ensureVisible(key('updates-install'));
    await tapAndSettle(tester, key('updates-install'));
    expect(fake.calls.sublist(fake.calls.length - 2), ['install updates/Scratch-Setup-0.1.0-prealpha.12.exe', 'exit']);
  });

  appTest('updates section: quiet offline check and a failed download', (tester) async {
    final (fake, u, _) = await openUpdates(tester, autoDownload: true);
    fake.next = const UpdateCheck(status: UpdateCheckStatus.offline, message: "Couldn't reach GitHub (offline)");
    await tapAndSettle(tester, key('updates-check-now'));
    expect(statusText(tester), 'Not checked yet.');
    expect(find.text("Couldn't reach GitHub (offline)"), findsOneWidget);

    fake.next = const UpdateCheck(status: UpdateCheckStatus.available, release: _release);
    fake.downloadError = 'The download of Scratch-Setup-0.1.0-prealpha.12.exe was corrupted (checksum mismatch) and was deleted';
    await tapAndSettle(tester, key('updates-check-now'));
    expect(fake.calls.last, 'download v0.1.0-prealpha.12', reason: 'auto download starts by itself');
    expect(u.phase, UpdatePhase.available);
    expect(find.textContaining('checksum mismatch'), findsOneWidget);
    expect(key('update-ready-row'), findsNothing);
    expect(fake.calls.where((c) => c.startsWith('install')), isEmpty);
  });

  appTest('updates section: nightly toggle re-checks with nightlies', (tester) async {
    final (fake, _, _) = await openUpdates(tester);
    await tapAndSettle(tester, key('updates-nightly'));
    expect(UpdateController.current!.settings.updatesIncludeNightly, isTrue);
    expect(fake.calls, ['check nightly=true']);
  });

  appTest('updates section: development build never checks', (tester) async {
    final (fake, _, _) = await openUpdates(tester, tag: '');
    expect(statusText(tester), 'Development build: updates are off.');
    expect(tester.widget<SSwitch>(key('updates-nightly')).onChanged, isNull);
    await tapAndSettle(tester, key('updates-check-now'));
    expect(fake.calls, isEmpty);
  });

  appTest('updates section is hidden off Windows', (tester) async {
    UpdateController.current = UpdateController(currentTag: 'v0.1.0', updatesDir: 'u', settings: AppSettings(), backend: FakeBackend());
    await pumpApp(tester, mobile: true);
    await openMobileSettings(tester);
    expect(find.byKey(const ValueKey('updates-check-now')), findsNothing);
    expect(find.text('Updates'), findsNothing);
  });
}
