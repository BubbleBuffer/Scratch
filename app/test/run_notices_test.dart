import 'dart:io';

import 'package:flutter/foundation.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/features/ai/ai_store.dart';
import 'package:notes_app/features/ai/run_notices.dart';
import 'package:notes_app/features/reminders/backends.dart';
import 'package:notes_app/features/reminders/reminder_plan.dart';
import 'package:notes_app/features/settings/settings_store.dart';

AiTask task(
  String id,
  String status, {
  String device = 'desktop',
  String error = '',
  int updatedMs = 2000,
  String? page,
}) => AiTask(
  id: id,
  prompt: 'p',
  page: page ?? 'note-$id',
  runner: 'agent',
  status: status,
  device: device,
  error: error,
  progress: '',
  progressAt: '',
  createdMs: 1000,
  updatedMs: updatedMs,
  node: 'desktop',
  project: '',
  worktreeBranch: '',
  worktreePath: '',
);

class _Source extends ChangeNotifier implements AiTaskSource {
  @override
  List<AiTask> tasks = [];
  @override
  bool get hubUsable => true;
  @override
  AiRunnerBridge get runnerBridge => throw UnimplementedError();
  @override
  Future<void> refresh() async {}
  @override
  void Function()? onDelegated;

  void set(List<AiTask> next) {
    tasks = next;
    notifyListeners();
  }
}

class _Backend extends ReminderBackend {
  final shown = <({int id, String title, String body, String payload})>[];
  bool granted = true;
  int permissionCalls = 0;

  @override
  Future<void> init() async {}
  @override
  Future<bool> ensurePermission() async {
    permissionCalls++;
    return granted;
  }

  @override
  Future<void> apply(List<PlannedReminder> plan) async {}
  @override
  Future<void> cancelAll() async {}
  @override
  Stream<String> get taps => const Stream.empty();
  @override
  Future<void> showNow({
    required int id,
    required String title,
    required String body,
    required String payload,
  }) async {
    shown.add((id: id, title: title, body: body, payload: payload));
  }
}

Future<void> settle() => Future<void>.delayed(Duration.zero);

void main() {
  group('runNoticeOf', () {
    test('finished and failed runs call for a notice, other states none', () {
      expect(runNoticeOf(task('a', 'done')), RunNoticeKind.finished);
      expect(runNoticeOf(task('a', 'failed')), RunNoticeKind.failed);
      expect(runNoticeOf(task('a', 'interrupted')), RunNoticeKind.failed);
      expect(runNoticeOf(task('a', 'queued')), isNull);
      expect(runNoticeOf(task('a', 'running')), isNull);
      expect(runNoticeOf(task('a', 'cancelled')), isNull);
      expect(runNoticeOf(task('a', 'something-new')), isNull);
    });
  });

  group('planRunNotices', () {
    test('the first look records the rows and notices none', () {
      final plan = planRunNotices(
        tasks: [task('a', 'done'), task('b', 'running')],
        seen: null,
        sinceMs: 0,
        deviceId: 'phone',
      );
      expect(plan.notify, isEmpty);
      expect(plan.seen, {'a': 'done'});
    });

    test('a newly ended run is noticed once', () {
      final first = planRunNotices(
        tasks: [task('a', 'done')],
        seen: {},
        sinceMs: 0,
        deviceId: 'phone',
      );
      expect(first.notify.map((t) => t.id), ['a']);
      final again = planRunNotices(
        tasks: [task('a', 'done')],
        seen: first.seen,
        sinceMs: 0,
        deviceId: 'phone',
      );
      expect(again.notify, isEmpty);
    });

    test('a retried run that ends again is noticed again', () {
      var seen = <String, String>{'a': 'failed'};
      seen = planRunNotices(
        tasks: [task('a', 'running')],
        seen: seen,
        sinceMs: 0,
        deviceId: 'phone',
      ).seen;
      expect(seen, isEmpty);
      final plan = planRunNotices(
        tasks: [task('a', 'done')],
        seen: seen,
        sinceMs: 0,
        deviceId: 'phone',
      );
      expect(plan.notify.map((t) => t.id), ['a']);
    });

    test('a change made by this device or before the first look is quiet', () {
      final plan = planRunNotices(
        tasks: [
          task('mine', 'done', device: 'phone'),
          task('old', 'done', updatedMs: 500),
          task('new', 'failed', updatedMs: 1500),
        ],
        seen: {},
        sinceMs: 1000,
        deviceId: 'phone',
      );
      expect(plan.notify.map((t) => t.id), ['new']);
      expect(plan.seen.keys, containsAll(['mine', 'old', 'new']));
    });

    test('a cancelled run gives no notice', () {
      final plan = planRunNotices(
        tasks: [task('a', 'cancelled')],
        seen: {},
        sinceMs: 0,
        deviceId: 'phone',
      );
      expect(plan.notify, isEmpty);
    });
  });

  test('a failed run body is its reason, one line', () {
    expect(
      runNoticeBody(
        RunNoticeKind.failed,
        task('a', 'failed', error: 'No key\nmore'),
      ),
      'No key',
    );
    expect(runNoticeBody(RunNoticeKind.finished, task('a', 'done')), '');
  });

  group('RunNotifier', () {
    late _Source source;
    late _Backend backend;
    late AppSettings settings;
    late RunNotifier notifier;

    RunNotifier make({bool enabledHere = true}) => RunNotifier(
      deviceId: 'phone',
      titleOf: (page) async => page == 'note-gone' ? null : 'Title of $page',
      settings: settings,
      backend: backend,
      source: source,
      enabledHere: enabledHere,
    );

    setUp(() {
      source = _Source();
      backend = _Backend();
      settings = AppSettings();
      settings.runNoticesSince = 0;
      notifier = make();
    });

    tearDown(() => notifier.dispose());

    test('posts a notice when a synced row ends, and opens its note', () async {
      source.tasks = [task('a', 'running')];
      await notifier.start();
      expect(backend.shown, isEmpty);

      source.set([task('a', 'done')]);
      await settle();
      expect(backend.shown, hasLength(1));
      expect(backend.shown.single.title, 'Task finished: Title of note-a');
      expect(backend.shown.single.payload, '{"page":"note-a"}');

      source.set([task('a', 'done', updatedMs: 3000)]);
      await settle();
      expect(backend.shown, hasLength(1));
    });

    test('a restart does not repeat a notice', () async {
      source.tasks = [task('a', 'running')];
      await notifier.start();
      source.set([task('a', 'failed', error: 'Out of tokens')]);
      await settle();
      expect(backend.shown, hasLength(1));
      expect(backend.shown.single.body, 'Out of tokens');
      notifier.dispose();

      notifier = make();
      await notifier.start();
      await settle();
      expect(backend.shown, hasLength(1));
    });

    test('with the setting off nothing is posted, and nothing repeats later',
        () async {
      settings.runNotices = false;
      source.tasks = [task('a', 'running')];
      await notifier.start();
      source.set([task('a', 'done')]);
      await settle();
      expect(backend.shown, isEmpty);
      settings.runNotices = true;
      source.set([task('a', 'done', updatedMs: 4000)]);
      await settle();
      expect(backend.shown, isEmpty);
    });

    test('a note that is gone gets no notice', () async {
      source.tasks = [task('a', 'running', page: 'note-gone')];
      await notifier.start();
      source.set([task('a', 'done', page: 'note-gone')]);
      await settle();
      expect(backend.shown, isEmpty);
    });

    test('without the permission nothing is posted', () async {
      backend.granted = false;
      source.tasks = [task('a', 'running')];
      await notifier.start();
      source.set([task('a', 'done')]);
      await settle();
      expect(backend.shown, isEmpty);
    });

    test('delegating a task asks for the permission while the setting is on',
        () async {
      await notifier.start();
      source.onDelegated?.call();
      await settle();
      expect(backend.permissionCalls, 1);
      settings.runNotices = false;
      source.onDelegated?.call();
      await settle();
      expect(backend.permissionCalls, 1);
    });

    test('off Android it does nothing', () async {
      final off = make(enabledHere: false);
      source.tasks = [task('a', 'running')];
      await off.start();
      source.set([task('a', 'done')]);
      await settle();
      expect(backend.shown, isEmpty);
      expect(source.onDelegated, isNull);
    });
  });

  test('settings keep the notice state across a reload', () {
    final dir = Directory.systemTemp.createTempSync('notes-run-notices-');
    try {
      final file = File('${dir.path}/settings.json');
      final s = AppSettings.load(file);
      expect(s.runNotices, isTrue);
      expect(s.runNoticesSeen, isNull);
      s.update((s) {
        s.runNotices = false;
        s.runNoticesSeen = {'a': 'done'};
        s.runNoticesSince = 1234;
      });
      final back = AppSettings.load(file);
      expect(back.runNotices, isFalse);
      expect(back.runNoticesSeen, {'a': 'done'});
      expect(back.runNoticesSince, 1234);
    } finally {
      dir.deleteSync(recursive: true);
    }
  });
}
