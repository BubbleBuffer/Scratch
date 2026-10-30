// The real runner backend against a fake runner bridge: the runner list, an
// assign that queues a task row, and each card's chip read from the newest
// task row about its page. The widget test shows a row's chip on a board.

import 'package:flutter/foundation.dart';
import 'package:flutter/widgets.dart' as w;
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/addons/kanban/api.dart';
import 'package:notes_app/ui/tokens.dart';
import 'package:notes_app/services/addon_services.dart'
    show AiRunnerBridge, AiTask, AiTaskSource;
import 'package:notes_app/src/rust/api/ai.dart'
    show AiRunner, AiRunnerKind, AiRunnerList, AiRunnerProject;

import 'kanban_fixture.dart';

final _now = DateTime(2026, 10, 24, 9);

AiRunnerList _fleet({String defaultId = 'studio'}) => AiRunnerList(
  defaultRunnerId: defaultId,
  runners: [
    AiRunner(
      projects: const [],
      id: 'hub',
      name: 'Hub',
      kind: AiRunnerKind.hub,
      online: true,
      lastSeenMs: null,
      isThisDevice: false,
    ),
    AiRunner(
      projects: const [
        AiRunnerProject(id: 'notes-app', name: 'notes-app'),
        AiRunnerProject(id: 'docs-site', name: 'docs-site'),
      ],
      id: 'studio',
      name: 'Studio-PC',
      kind: AiRunnerKind.pc,
      online: true,
      lastSeenMs: _now.millisecondsSinceEpoch,
      isThisDevice: true,
    ),
    AiRunner(
      projects: const [],
      id: 'laptop',
      name: 'Laptop',
      kind: AiRunnerKind.pc,
      online: false,
      lastSeenMs: _now.subtract(const Duration(hours: 2)).millisecondsSinceEpoch,
      isThisDevice: false,
    ),
  ],
);

AiTask _row(
  String id,
  String page,
  String status, {
  String node = 'studio',
  String error = '',
  String progress = '',
  String project = '',
  bool cancelRequested = false,
  bool stale = false,
}) => AiTask(
  project: project,
  worktreeBranch: '',
  worktreePath: '',
  id: id,
  prompt: runnerCardPrompt,
  page: page,
  runner: 'agent',
  status: status,
  device: '',
  error: error,
  progress: progress,
  progressAt: '',
  createdMs: 0,
  updatedMs: 0,
  node: node,
  cancelRequested: cancelRequested,
  stale: stale,
);

/// Records the calls and answers from fixed data.
class _FakeBridge implements AiRunnerBridge {
  _FakeBridge(this.list);

  AiRunnerList list;
  final calls = <String>[];
  AiTask? assigned;
  String? failure;

  @override
  Future<AiRunnerList> listRunners({required bool hubUsable}) async {
    calls.add('list:$hubUsable');
    if (failure != null) throw failure!;
    return list;
  }

  @override
  Future<AiTask> assign({
    required String prompt,
    required String page,
    required String runnerId,
    String? projectId,
  }) async {
    final project = projectId == null ? '' : ':$projectId';
    calls.add('assign:$page:$runnerId:${prompt == runnerCardPrompt}$project');
    return assigned!;
  }

  @override
  Future<void> rename({required String deviceId, required String name}) async {
    calls.add('rename:$deviceId:$name');
  }
}

/// A task source whose rows the test sets; notifying it is what the AI store
/// does when rows sync.
class _FakeSource extends ChangeNotifier implements AiTaskSource {
  _FakeSource(this.runnerBridge);

  @override
  final _FakeBridge runnerBridge;
  @override
  List<AiTask> tasks = [];
  @override
  bool hubUsable = true;
  @override
  void Function()? onDelegated;
  int refreshes = 0;
  final cancelled = <String>[];
  final updated = <String>[];

  @override
  Future<void> cancel(String id) async => cancelled.add(id);

  @override
  Future<void> sendUpdate(String id) async => updated.add(id);

  @override
  Future<void> refresh() async {
    refreshes++;
    notifyListeners();
  }
}

void main() {
  group('real runner backend', () {
    test('lists the hub first, then the PCs, with the default and this device marked', () async {
      final bridge = _FakeBridge(_fleet());
      final backend = RealRunnerBackend(_FakeSource(bridge), clock: () => _now);
      final runners = await backend.listRunners();

      expect(runners.map((r) => r.name), ['Hub', 'Studio-PC', 'Laptop']);
      expect(runners[0].kind, RunnerKind.hub);
      expect(runners[1].kind, RunnerKind.pc);
      expect(runners[1].isDefault, isTrue);
      expect(runners[1].isThisDevice, isTrue);
      expect(runners[2].online, isFalse);
      expect(runners[2].lastSeen, _now.subtract(const Duration(hours: 2)));
      expect(bridge.calls, ['list:true']);
    });

    test('a queued row on an online runner is queued, and on an offline runner it is waiting with the reason', () async {
      final source = _FakeSource(_FakeBridge(_fleet()));
      final backend = RealRunnerBackend(source, clock: () => _now);
      await backend.listRunners();

      source.tasks = [_row('t1', 'card-a', 'queued', node: 'studio')];
      final online = backend.current('card-a')!;
      expect(online.state, AssignState.queued);
      expect(online.runnerName, 'Studio-PC');
      expect(online.reason, isNull);

      source.tasks = [_row('t2', 'card-b', 'queued', node: 'laptop')];
      final waiting = backend.current('card-b')!;
      expect(waiting.state, AssignState.waiting);
      expect(waiting.runnerName, 'Laptop');
      expect(waiting.reason, 'Laptop is offline (last seen 2 h ago)');
    });

    test('running, done, failed with its error, cancelled and interrupted rows map to their chips', () async {
      final source = _FakeSource(_FakeBridge(_fleet()));
      final backend = RealRunnerBackend(source, clock: () => _now);
      await backend.listRunners();

      source.tasks = [_row('t1', 'card-run', 'running')];
      expect(backend.current('card-run')!.state, AssignState.running);

      source.tasks = [_row('t2', 'card-done', 'done')];
      expect(backend.current('card-done')!.state, AssignState.done);

      source.tasks = [_row('t3', 'card-fail', 'failed', error: 'The model stopped.')];
      final failed = backend.current('card-fail')!;
      expect(failed.state, AssignState.failed);
      expect(failed.reason, 'The model stopped.');

      source.tasks = [_row('t4', 'card-cancel', 'cancelled')];
      expect(backend.current('card-cancel')!.state, AssignState.cancelled);

      source.tasks = [_row('t5', 'card-int', 'interrupted')];
      final interrupted = backend.current('card-int')!;
      expect(interrupted.state, AssignState.failed);
      expect(interrupted.reason, 'The run was interrupted before it finished.');
    });

    test('a card reads its newest row, and a card with no row has no assignment', () async {
      final source = _FakeSource(_FakeBridge(_fleet()));
      final backend = RealRunnerBackend(source, clock: () => _now);
      await backend.listRunners();

      // The rows are newest first.
      source.tasks = [
        _row('t9', 'card-a', 'done'),
        _row('t1', 'card-a', 'failed', error: 'old'),
      ];
      expect(backend.current('card-a')!.state, AssignState.done);
      expect(backend.current('card-none'), isNull);
    });

    test('assign queues the task for the chosen runner and asks the store to refresh', () async {
      final bridge = _FakeBridge(_fleet())
        ..assigned = _row('t5', 'card-c', 'queued', node: 'laptop');
      final source = _FakeSource(bridge);
      final backend = RealRunnerBackend(source, clock: () => _now);

      final a = await backend.assign('card-c', 'laptop');

      expect(bridge.calls, contains('assign:card-c:laptop:true'));
      expect(a.taskId, 'card-c');
      expect(a.runnerId, 'laptop');
      expect(a.state, AssignState.waiting);
      expect(a.reason, 'Laptop is offline (last seen 2 h ago)');
      expect(source.refreshes, 1);
    });

    test('lists each PC with its projects, and the hub with none', () async {
      final backend = RealRunnerBackend(
        _FakeSource(_FakeBridge(_fleet())),
        clock: () => _now,
      );
      final runners = await backend.listRunners();

      expect(runners[0].projects, isEmpty);
      expect(runners[1].projects.map((p) => p.id), ['notes-app', 'docs-site']);
      expect(runners[1].projects.map((p) => p.name), ['notes-app', 'docs-site']);
      expect(runners[2].projects, isEmpty);
    });

    test('assign passes the chosen project to the bridge, and the chip names it', () async {
      final bridge = _FakeBridge(_fleet())
        ..assigned = _row(
          't7',
          'card-d',
          'queued',
          node: 'studio',
          project: 'docs-site',
        );
      final source = _FakeSource(bridge);
      final backend = RealRunnerBackend(source, clock: () => _now);
      await backend.listRunners();

      final a = await backend.assign('card-d', 'studio', projectId: 'docs-site');

      expect(bridge.calls, contains('assign:card-d:studio:true:docs-site'));
      expect(a.projectName, 'docs-site');
      expect(runnerChipText(a), 'Studio-PC · docs-site · queued');

      source.tasks = [bridge.assigned!];
      expect(backend.current('card-d')!.projectName, 'docs-site');
    });

    test('a task with no project, or a project the listing lacks, names none', () async {
      final bridge = _FakeBridge(_fleet())
        ..assigned = _row('t8', 'card-e', 'queued', node: 'studio');
      final source = _FakeSource(bridge)
        ..tasks = [
          _row('t9', 'card-f', 'queued', node: 'studio', project: 'gone'),
        ];
      final backend = RealRunnerBackend(source, clock: () => _now);
      await backend.listRunners();

      final none = await backend.assign('card-e', 'studio');
      expect(bridge.calls, contains('assign:card-e:studio:true'));
      expect(none.projectName, isNull);
      expect(runnerChipText(none), 'Studio-PC · queued');
      expect(backend.current('card-f')!.projectName, isNull);
    });

    test('a runner list that fails reports the reason as RunnerUnavailable', () async {
      final bridge = _FakeBridge(_fleet())..failure = 'The vault is locked.';
      final backend = RealRunnerBackend(_FakeSource(bridge), clock: () => _now);

      await expectLater(
        backend.listRunners(),
        throwsA(
          isA<RunnerUnavailable>().having(
            (e) => e.reason,
            'reason',
            contains('The vault is locked.'),
          ),
        ),
      );
    });

    test('the store notifying redraws the chip state without a picker call', () async {
      final source = _FakeSource(_FakeBridge(_fleet()));
      final backend = RealRunnerBackend(source, clock: () => _now);
      final assignments = RunnerAssignments(backend);
      await backend.listRunners();

      var notes = 0;
      assignments.addListener(() => notes++);
      source.tasks = [_row('t1', 'card-a', 'running')];
      source.notifyListeners();

      expect(notes, greaterThan(0));
      expect(assignments.of('card-a')!.state, AssignState.running);
    });
  });

  group('progress line', () {
    RunnerAssignment at(AssignState s, {String? progress}) => RunnerAssignment(
      taskId: 't',
      runnerId: 'studio',
      runnerName: 'Studio-PC',
      state: s,
      progress: progress,
    );

    test('shows while running and after a failed or cancelled run, never after a done run', () {
      expect(runnerProgressText(at(AssignState.running, progress: 'Reading')), 'Reading');
      expect(runnerProgressText(at(AssignState.failed, progress: 'Reading')), 'Reading');
      expect(runnerProgressText(at(AssignState.cancelled, progress: 'Reading')), 'Reading');
      expect(runnerProgressText(at(AssignState.done, progress: 'Reading')), isNull);
      expect(runnerProgressText(at(AssignState.queued, progress: 'Reading')), isNull);
      expect(runnerProgressText(at(AssignState.waiting, progress: 'Reading')), isNull);
    });

    test('is trimmed, and blank or unset text shows nothing', () {
      expect(runnerProgressText(at(AssignState.running, progress: '  Reading  ')), 'Reading');
      expect(runnerProgressText(at(AssignState.running, progress: '   ')), isNull);
      expect(runnerProgressText(at(AssignState.running)), isNull);
    });

    test('the real backend maps the row progress onto the assignment', () async {
      final source = _FakeSource(_FakeBridge(_fleet()));
      final backend = RealRunnerBackend(source, clock: () => _now);
      await backend.listRunners();

      source.tasks = [_row('t1', 'card-p', 'running', progress: '  Reading the plan  ')];
      expect(backend.current('card-p')!.progress, 'Reading the plan');

      source.tasks = [_row('t2', 'card-q', 'running')];
      expect(backend.current('card-q')!.progress, isNull);
    });

    test('the demo runs its first online assignment with sample progress, then queues', () async {
      final backend = FakeRunnerBackend.demo(now: _now, sampleRunning: true);
      final first = await backend.assign('card-1', 'office');
      expect(first.state, AssignState.running);
      expect(first.progress, sampleRunnerProgress);
      final second = await backend.assign('card-2', 'office');
      expect(second.state, AssignState.queued);
      expect(second.progress, isNull);
    });
  });

  setUpAll(initCore);

  appTest('a board card shows its agent row as a chip, with the waiting reason', (
    tester,
  ) async {
    final bridge = _FakeBridge(_fleet());
    final source = _FakeSource(bridge);
    final runners = aiRunnerAssignments(source);
    final f = await pumpApp(tester, mobile: false, runners: runners);
    final garden = f.pages['Garden plan']!;
    final board = await seedKanban(
      tester,
      title: 'Launch board',
      columns: {
        'To do': [garden],
        'Doing': [],
        'Done': [],
      },
    );
    await openNote(tester, board, 'Launch board');

    source.tasks = [_row('t1', garden, 'queued', node: 'laptop')];
    source.notifyListeners();
    await settle(tester);

    expect(find.byKey(ValueKey('runner-chip-$garden')), findsOneWidget);
    expect(find.text('Laptop · waiting'), findsOneWidget);
    expect(
      find.textContaining('Laptop is offline (last seen'),
      findsOneWidget,
    );
  });

  appTest('the progress line shows under a running chip, quietly after a failed run, and not after a done run', (
    tester,
  ) async {
    final source = _FakeSource(_FakeBridge(_fleet()));
    final runners = aiRunnerAssignments(source);
    final f = await pumpApp(tester, mobile: false, runners: runners);
    final running = f.pages['Garden plan']!;
    final failed = f.pages['Budget']!;
    final done = f.pages['Recipes']!;
    final board = await seedKanban(
      tester,
      title: 'Progress board',
      columns: {
        'To do': [running, failed, done],
        'Doing': [],
        'Done': [],
      },
    );
    await openNote(tester, board, 'Progress board');

    source.tasks = [
      _row('t3', done, 'done', progress: 'Wrote the summary'),
      _row('t2', failed, 'failed', progress: 'Checking totals', error: 'The model stopped.'),
      _row('t1', running, 'running', progress: 'Reading the plan'),
    ];
    source.notifyListeners();
    await settle(tester);

    final runningLine = find.byKey(ValueKey('runner-progress-$running'));
    expect(runningLine, findsOneWidget);
    expect(tester.widget<w.Text>(runningLine).data, 'Reading the plan');
    expect(
      tester.widget<w.Text>(runningLine).style?.color,
      tester.element(runningLine).colors.textSecondary,
    );

    final failedLine = find.byKey(ValueKey('runner-progress-$failed'));
    expect(failedLine, findsOneWidget);
    expect(tester.widget<w.Text>(failedLine).data, 'Checking totals');
    expect(
      tester.widget<w.Text>(failedLine).style?.color,
      tester.element(failedLine).colors.textTertiary,
    );

    expect(find.byKey(ValueKey('runner-progress-$done')), findsNothing);
    expect(find.text('Wrote the summary'), findsNothing);
  });
}
