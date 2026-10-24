// The fake runner backend: demo runners and an in-memory assignment per task.
// Used with --dart-define=SEED_DEMO=true and by the widget tests. It runs
// nothing, so an assignment to an online runner stays `queued` until a test
// or a later engine records the next state.

import 'runner_backend.dart';

/// How long ago [seen] was, as the picker words it: "2 h ago".
String lastSeenText(DateTime seen, DateTime now) {
  final gap = now.difference(seen);
  if (gap.inMinutes < 1) return 'just now';
  if (gap.inHours < 1) return '${gap.inMinutes} min ago';
  if (gap.inDays < 1) return '${gap.inHours} h ago';
  return '${gap.inDays} d ago';
}

class FakeRunnerBackend extends RunnerBackend {
  FakeRunnerBackend({required this.runners, DateTime? now})
    : now = now ?? DateTime.now();

  /// The demo runners: the hub, Studio-PC (this device and the default),
  /// Office-PC online, and Laptop offline for two hours.
  factory FakeRunnerBackend.demo({DateTime? now}) {
    final at = now ?? DateTime.now();
    return FakeRunnerBackend(
      now: at,
      runners: [
        const Runner(
          id: 'hub',
          name: 'Hub',
          kind: RunnerKind.hub,
          online: true,
        ),
        Runner(
          id: 'studio',
          name: 'Studio-PC',
          kind: RunnerKind.pc,
          online: true,
          lastSeen: at,
          isThisDevice: true,
          isDefault: true,
        ),
        Runner(
          id: 'office',
          name: 'Office-PC',
          kind: RunnerKind.pc,
          online: true,
          lastSeen: at,
        ),
        Runner(
          id: 'laptop',
          name: 'Laptop',
          kind: RunnerKind.pc,
          online: false,
          lastSeen: at.subtract(const Duration(hours: 2)),
        ),
      ],
    );
  }

  final List<Runner> runners;

  /// The clock the reasons are worded from.
  final DateTime now;

  /// Calls in order, as `name:args`, for assertions.
  final List<String> calls = [];

  /// When set, every call fails with this reason.
  String? error;

  @override
  Future<List<Runner>> listRunners() async {
    calls.add('list');
    final failure = error;
    if (failure != null) throw RunnerUnavailable(failure);
    return List.of(runners);
  }

  @override
  Future<RunnerAssignment> assign(String taskId, String runnerId) async {
    calls.add('assign:$taskId:$runnerId');
    final failure = error;
    if (failure != null) throw RunnerUnavailable(failure);
    Runner? runner;
    for (final r in runners) {
      if (r.id == runnerId) runner = r;
    }
    if (runner == null) throw const RunnerUnavailable('Unknown runner.');
    if (runner.online) {
      return RunnerAssignment(
        taskId: taskId,
        runnerId: runner.id,
        runnerName: runner.name,
        state: AssignState.queued,
      );
    }
    final seen = runner.lastSeen;
    final since = seen == null ? '' : ' (last seen ${lastSeenText(seen, now)})';
    return RunnerAssignment(
      taskId: taskId,
      runnerId: runner.id,
      runnerName: runner.name,
      state: AssignState.waiting,
      reason: '${runner.name} is offline$since',
    );
  }
}

/// The demo's assignments for one app run. The board reads it only when the
/// app runs with SEED_DEMO and no RunnerScope is given.
final RunnerAssignments demoRunnerAssignments = RunnerAssignments(
  FakeRunnerBackend.demo(),
);
