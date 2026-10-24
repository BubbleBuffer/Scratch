// The runner picker's fake backend and state holder, without any widgets:
// the demo runners, what an assignment answers for each kind of runner, the
// reason wording, the chip text and the notifications cards listen to.

import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/addons/kanban/api.dart';

void main() {
  final now = DateTime(2026, 10, 24, 9);

  group('demo runners', () {
    test('lists the hub, this device as the default, an online PC and an offline laptop', () async {
      final backend = FakeRunnerBackend.demo(now: now);
      final runners = await backend.listRunners();

      expect(runners.map((r) => r.name), ['Hub', 'Studio-PC', 'Office-PC', 'Laptop']);
      expect(runners[0].kind, RunnerKind.hub);
      expect(runners[1].isThisDevice, isTrue);
      expect(runners[1].isDefault, isTrue);
      expect(runners[2].online, isTrue);
      expect(runners[3].online, isFalse);
      expect(runners[3].lastSeen, now.subtract(const Duration(hours: 2)));
      expect(backend.calls, ['list']);
    });

    test('an assignment to an online runner is queued and carries no reason', () async {
      final backend = FakeRunnerBackend.demo(now: now);
      final a = await backend.assign('task-1', 'office');

      expect(a.taskId, 'task-1');
      expect(a.runnerId, 'office');
      expect(a.runnerName, 'Office-PC');
      expect(a.state, AssignState.queued);
      expect(a.reason, isNull);
      expect(backend.calls, ['assign:task-1:office']);
    });

    test('an assignment to an offline runner waits, with the reason and how long ago it was seen', () async {
      final backend = FakeRunnerBackend.demo(now: now);
      final a = await backend.assign('task-1', 'laptop');

      expect(a.state, AssignState.waiting);
      expect(a.reason, 'Laptop is offline (last seen 2 h ago)');
    });

    test('an unknown runner is refused', () async {
      final backend = FakeRunnerBackend.demo(now: now);
      await expectLater(
        backend.assign('task-1', 'nope'),
        throwsA(isA<RunnerUnavailable>().having((e) => e.reason, 'reason', 'Unknown runner.')),
      );
    });

    test('when the backend is down every call fails with its reason', () async {
      final backend = FakeRunnerBackend.demo(now: now)..error = 'The hub is not reachable.';
      await expectLater(
        backend.listRunners(),
        throwsA(isA<RunnerUnavailable>().having((e) => e.reason, 'reason', 'The hub is not reachable.')),
      );
      await expectLater(
        backend.assign('task-1', 'office'),
        throwsA(isA<RunnerUnavailable>()),
      );
    });
  });

  group('the last-seen wording in the waiting reason', () {
    Future<String?> reasonSeen(DateTime seen) async {
      final backend = FakeRunnerBackend(
        now: now,
        runners: [
          Runner(id: 'x', name: 'X', kind: RunnerKind.pc, online: false, lastSeen: seen),
        ],
      );
      return (await backend.assign('t', 'x')).reason;
    }

    test('minutes, hours and days', () async {
      expect(
        await reasonSeen(now.subtract(const Duration(seconds: 20))),
        'X is offline (last seen just now)',
      );
      expect(
        await reasonSeen(now.subtract(const Duration(minutes: 30))),
        'X is offline (last seen 30 min ago)',
      );
      expect(
        await reasonSeen(now.subtract(const Duration(hours: 5))),
        'X is offline (last seen 5 h ago)',
      );
      expect(
        await reasonSeen(now.subtract(const Duration(days: 3))),
        'X is offline (last seen 3 d ago)',
      );
    });
  });

  group('chip text', () {
    test('is the runner name, a dot and the state, for every state', () {
      const names = {
        AssignState.queued: 'queued',
        AssignState.waiting: 'waiting',
        AssignState.running: 'running',
        AssignState.cancelling: 'cancelling',
        AssignState.done: 'done',
        AssignState.failed: 'failed',
      };
      expect(names.keys.toSet(), AssignState.values.toSet(), reason: 'every state has a chip word');
      for (final entry in names.entries) {
        final a = RunnerAssignment(
          taskId: 't',
          runnerId: 'laptop',
          runnerName: 'Laptop',
          state: entry.key,
        );
        expect(runnerChipText(a), 'Laptop · ${entry.value}');
      }
    });
  });

  group('RunnerAssignments', () {
    test('keeps the latest assignment per task and tells its listeners', () async {
      final runners = RunnerAssignments(FakeRunnerBackend.demo(now: now));
      var notified = 0;
      runners.addListener(() => notified++);

      expect(runners.of('task-1'), isNull);

      final queued = await runners.assign('task-1', 'office');
      expect(runners.of('task-1'), same(queued));
      expect(notified, 1);

      runners.record(
        const RunnerAssignment(
          taskId: 'task-1',
          runnerId: 'office',
          runnerName: 'Office-PC',
          state: AssignState.done,
        ),
      );
      expect(runners.of('task-1')!.state, AssignState.done);
      expect(runners.of('task-2'), isNull, reason: 'other tasks are untouched');
      expect(notified, 2);
    });

    test('lists the runners through its backend', () async {
      final runners = RunnerAssignments(FakeRunnerBackend.demo(now: now));
      expect((await runners.runners()).length, 4);
    });
  });
}
