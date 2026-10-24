// The real runner backend against a fake runner bridge: the runner list, an
// assign that queues a task row, and each card's chip read from the newest
// task row about its page. The widget test shows a row's chip on a board.

import 'package:flutter/foundation.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/addons/kanban/api.dart';
import 'package:notes_app/services/addon_services.dart'
    show AiRunnerBridge, AiTask, AiTaskSource;
import 'package:notes_app/src/rust/api/ai.dart'
    show AiRunner, AiRunnerKind, AiRunnerList;

import 'kanban_fixture.dart';

final _now = DateTime(2026, 10, 24, 9);

AiRunnerList _fleet({String defaultId = 'studio'}) => AiRunnerList(
  defaultRunnerId: defaultId,
  runners: [
    AiRunner(
      id: 'hub',
      name: 'Hub',
      kind: AiRunnerKind.hub,
      online: true,
      lastSeenMs: null,
      isThisDevice: false,
    ),
    AiRunner(
      id: 'studio',
      name: 'Studio-PC',
      kind: AiRunnerKind.pc,
      online: true,
      lastSeenMs: _now.millisecondsSinceEpoch,
      isThisDevice: true,
    ),
    AiRunner(
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
}) => AiTask(
  id: id,
  prompt: runnerCardPrompt,
  page: page,
  runner: 'agent',
  status: status,
  device: '',
  error: error,
  progress: '',
  progressAt: '',
  createdMs: 0,
  updatedMs: 0,
  node: node,
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
  }) async {
    calls.add('assign:$page:$runnerId:${prompt == runnerCardPrompt}');
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
  int refreshes = 0;

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
}
