// The fake runner backend: demo runners and an in-memory assignment per task.
// Used with --dart-define=SEED_DEMO=true and by the widget tests. It runs
// nothing, so an assignment to an online runner stays `queued` until a test
// or a later engine records the next state. The SEED_DEMO build (sampleRunning)
// makes its first online assignment run with sample progress text instead.

import 'runner_backend.dart';

/// The progress text the demo's sample running task shows.
const sampleRunnerProgress = 'Reading the note and the pages it links to';

class FakeRunnerBackend extends RunnerBackend {
  FakeRunnerBackend({
    required this.runners,
    DateTime? now,
    this.sampleRunning = false,
  }) : now = now ?? DateTime.now();

  /// The demo runners: the hub, Studio-PC (this device and the default),
  /// Office-PC online, and Laptop offline for two hours. With [sampleRunning],
  /// the first online assignment runs with sample progress.
  factory FakeRunnerBackend.demo({DateTime? now, bool sampleRunning = false}) {
    final at = now ?? DateTime.now();
    return FakeRunnerBackend(
      now: at,
      sampleRunning: sampleRunning,
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

  /// When set, the first assignment to an online runner runs with
  /// [sampleRunnerProgress] instead of queuing.
  final bool sampleRunning;
  bool _sampled = false;

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
      final sample = sampleRunning && !_sampled;
      if (sample) _sampled = true;
      return RunnerAssignment(
        taskId: taskId,
        runnerId: runner.id,
        runnerName: runner.name,
        state: sample ? AssignState.running : AssignState.queued,
        progress: sample ? sampleRunnerProgress : null,
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
  FakeRunnerBackend.demo(sampleRunning: true),
);
