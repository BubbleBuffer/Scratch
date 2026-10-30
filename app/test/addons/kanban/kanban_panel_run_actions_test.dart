// The panel's actions on a run: Accept on a result waiting for review, Mark
// failed on a stalled run, Add step on a running one. The card moves between
// columns in the task layer (crates/mcp), not here.

import 'package:flutter/foundation.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/addons/kanban/api.dart';
import 'package:flutter/widgets.dart';

import 'kanban_fixture.dart';

class _Ping extends ChangeNotifier {
  void ping() => notifyListeners();
}

class _Backend extends FakeRunnerBackend {
  _Backend(this.state) : super(runners: const []);

  AssignState state;
  final calls = <String>[];
  final _changes = _Ping();

  @override
  Listenable? get changes => _changes;

  void set(AssignState next) {
    state = next;
    _changes.ping();
  }

  @override
  RunnerAssignment? current(String taskId) => RunnerAssignment(
    taskId: taskId,
    runnerId: 'studio',
    runnerName: 'Studio-PC',
    state: state,
  );

  @override
  Future<void> accept(String taskId) async => calls.add('accept');

  @override
  Future<void> markFailed(String taskId) async => calls.add('failed');

  @override
  Future<void> addStep(String taskId, String text) async =>
      calls.add('step:$text');
}

Future<_Backend> _open(
  WidgetTester tester,
  AssignState state,
  Map<String, List<String>> Function(String garden) columns,
) async {
  final backend = _Backend(state);
  final f = await pumpApp(tester, mobile: false, runners: RunnerAssignments(backend));
  final garden = f.pages['Garden plan']!;
  final board = await seedKanban(
    tester,
    title: 'Launch board',
    columns: columns(garden),
  );
  await openNote(tester, board, 'Launch board');
  return backend;
}

void main() {
  setUpAll(initCore);

  appTest('Accept in the panel accepts the run', (
    tester,
  ) async {
    final backend = await _open(
      tester,
      AssignState.review,
      (g) => {'To do': [g], 'Doing': [], 'Done': []},
    );
    await tapAndSettle(tester, cardIn('To do', 'Garden plan'));
    await tapAndSettle(tester, find.byKey(const ValueKey('kanban-panel-accept')));
    expect(backend.calls, ['accept']);
  });

  appTest('Mark failed shows on a stalled run, Add step on a running one', (
    tester,
  ) async {
    final backend = await _open(
      tester,
      AssignState.stalled,
      (g) => {'To do': [g], 'Done': []},
    );
    await tapAndSettle(tester, cardIn('To do', 'Garden plan'));
    expect(find.byKey(const ValueKey('kanban-panel-add-step')), findsNothing);
    await tapAndSettle(tester, find.byKey(const ValueKey('kanban-panel-mark-failed')));
    expect(backend.calls, ['failed']);

    backend.set(AssignState.running);
    await settle(tester);
    expect(find.byKey(const ValueKey('kanban-panel-mark-failed')), findsNothing);
    await tester.enterText(
      find.byKey(const ValueKey('kanban-panel-add-step')),
      'Check the logs',
    );
    await tester.testTextInput.receiveAction(TextInputAction.done);
    await settle(tester);
    expect(backend.calls, ['failed', 'step:Check the logs']);
  });
}
