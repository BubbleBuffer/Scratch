// The Triage UI against a fake bridge: the proposals list (titles, reason,
// count), Merge / Dismiss / Undo, the empty state, opening a page, reload on a
// change to the proposals page, and the Triage action with a picked runner.

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/features/ai/ai_runners.dart';
import 'package:notes_app/features/triage/proposals_panel.dart';
import 'package:notes_app/features/triage/triage_store.dart';
import 'package:notes_app/features/triage/triage_start.dart';
import 'package:notes_app/src/rust/api/ai.dart'
    show AiRunner, AiRunnerKind, AiRunnerList, AiTask;
import 'package:notes_app/src/rust/api/types.dart'
    show Change, ChangeBatch, ChangeKind;
import 'package:notes_app/ui/ui.dart';

TriageProposalDto _p(
  String id, {
  String status = 'pending',
  String keep = 'Garden plan',
  String dropped = 'Garden plan copy',
  String reason = 'Same list of beds',
  String? version,
}) => TriageProposalDto(
  id: id,
  keepId: 'k-$id',
  keepTitle: keep,
  droppedId: 'd-$id',
  droppedTitle: dropped,
  pageType: 'document',
  reason: reason,
  status: status,
  createdMs: 1,
  task: '',
  mergedVersion: version,
);

class FakeTriage implements TriageBridge {
  FakeTriage(this.items, {List<TriageRunDto>? runs}) : runs = runs ?? [];
  List<TriageProposalDto> items;
  List<TriageRunDto> runs;
  final calls = <String>[];

  void _set(String id, String status) => items = [
    for (final p in items)
      if (p.id == id)
        TriageProposalDto(
          id: p.id,
          keepId: p.keepId,
          keepTitle: p.keepTitle,
          droppedId: p.droppedId,
          droppedTitle: p.droppedTitle,
          pageType: p.pageType,
          reason: p.reason,
          status: status,
          createdMs: p.createdMs,
          task: p.task,
          mergedVersion: status == 'accepted' ? 'v1' : null,
        )
      else
        p,
  ];

  TriageProposalDto _get(String id) => items.firstWhere((p) => p.id == id);

  @override
  Future<List<TriageProposalDto>> all() async => items;
  @override
  Future<TriageProposalDto> accept(String id) async {
    calls.add('accept:$id');
    _set(id, 'accepted');
    return _get(id);
  }

  @override
  Future<TriageProposalDto> dismiss(String id) async {
    calls.add('dismiss:$id');
    _set(id, 'dismissed');
    return _get(id);
  }

  @override
  Future<TriageProposalDto> undo(String id) async {
    calls.add('undo:$id');
    _set(id, 'dismissed');
    return _get(id);
  }

  @override
  Future<List<TriageRunDto>> applied() async => runs;
  @override
  Future<int> undoApplied(String task) async {
    calls.add('undo-applied:$task');
    final n = runs.firstWhere((r) => r.task == task).actions.length;
    runs = [
      for (final r in runs)
        if (r.task != task) r,
    ];
    return n;
  }

  @override
  Future<String> pageId() async => 'nodes';

  @override
  Future<String> start({
    String? projectId,
    List<String> pageIds = const [],
    required String runner,
  }) async {
    calls.add('start:$projectId:$runner');
    return 'task';
  }
}

class FakeRunners implements AiRunnerBridge {
  @override
  Future<AiRunnerList> listRunners({required bool hubUsable}) async =>
      AiRunnerList(
        defaultRunnerId: 'studio',
        runners: [
          AiRunner(
            projects: [],
            id: 'hub',
            name: 'Hub',
            kind: AiRunnerKind.hub,
            online: true,
            isThisDevice: false,
          ),
          AiRunner(
            projects: [],
            id: 'studio',
            name: 'Studio-PC',
            kind: AiRunnerKind.pc,
            online: true,
            isThisDevice: true,
          ),
          AiRunner(
            projects: [],
            id: 'laptop',
            name: 'Laptop',
            kind: AiRunnerKind.pc,
            online: false,
            isThisDevice: false,
          ),
        ],
      );

  @override
  Future<AiTask> assign({
    required String prompt,
    required String page,
    required String runnerId,
    String? projectId,
  }) => throw UnimplementedError();

  @override
  Future<void> rename({required String deviceId, required String name}) =>
      throw UnimplementedError();
}

Future<void> _mount(
  WidgetTester tester,
  Widget Function(BuildContext) home, {
  bool mobile = false,
}) async {
  tester.view.physicalSize = mobile ? const Size(390, 800) : const Size(1100, 700);
  tester.view.devicePixelRatio = 1;
  addTearDown(tester.view.resetPhysicalSize);
  addTearDown(tester.view.resetDevicePixelRatio);
  await tester.pumpWidget(
    AppTheme(
      colors: AppColors.light,
      formFactor: mobile ? FormFactor.mobile : FormFactor.desktop,
      child: WidgetsApp(
        color: const Color(0xFFFFFFFF),
        onGenerateRoute: (_) => PageRouteBuilder<void>(
          pageBuilder: (context, a, b) => home(context),
        ),
      ),
    ),
  );
  await tester.pumpAndSettle();
}

Future<TriageStore> _panel(
  WidgetTester tester,
  FakeTriage fake, {
  List<String>? opened,
  bool mobile = false,
}) async {
  final store = TriageStore(fake);
  addTearDown(store.dispose);
  await _mount(
    tester,
    (_) => ProposalsPanel(
      store: store,
      onOpenPage: (id, title) => opened?.add('$id:$title'),
    ),
    mobile: mobile,
  );
  return store;
}

Finder _k(String key) => find.byKey(ValueKey(key));

void main() {
  testWidgets('lists both titles, the reason and the consequence; counts pending', (
    tester,
  ) async {
    final fake = FakeTriage([
      _p('a'),
      _p('b', keep: 'Trip', dropped: 'Trip notes', reason: 'Both plan the trip'),
      _p('c', status: 'accepted'),
      _p('x', status: 'dismissed'),
    ]);
    final store = await _panel(tester, fake);

    expect(find.text('Garden plan'), findsWidgets);
    expect(find.text('Garden plan copy'), findsWidgets);
    expect(find.text('Same list of beds'), findsWidgets);
    expect(find.text('Both plan the trip'), findsOneWidget);
    expect(
      find.text(
        "Merge: keeps 'Trip', moves the content of 'Trip notes' into it and "
        "moves 'Trip notes' to trash. Undo is in page history.",
      ),
      findsOneWidget,
    );
    expect(store.pending, 2, reason: 'the badge counts pending only');
    expect(_k('proposal-x'), findsNothing, reason: 'dismissed is hidden');
    expect(_k('proposal-c'), findsOneWidget, reason: 'merged stays for Undo');
  });

  testWidgets('Merge calls accept and the entry then says Merged with Undo', (
    tester,
  ) async {
    final fake = FakeTriage([_p('a')]);
    final store = await _panel(tester, fake);

    await tester.tap(_k('proposal-merge-a'));
    await tester.pumpAndSettle();

    expect(fake.calls, ['accept:a']);
    expect(find.text('Merged'), findsOneWidget);
    expect(_k('proposal-undo-a'), findsOneWidget);
    expect(_k('proposal-merge-a'), findsNothing);
    expect(store.pending, 0);

    await tester.tap(_k('proposal-undo-a'));
    await tester.pumpAndSettle();
    expect(fake.calls, ['accept:a', 'undo:a']);
    expect(_k('proposal-a'), findsNothing);
    expect(find.text('No proposals. Run Triage on a project to look for duplicates.'), findsOneWidget);
  });

  testWidgets('lists the links and tags of a run and Undo takes them back', (
    tester,
  ) async {
    final fake = FakeTriage(
      [],
      runs: [
        TriageRunDto(
          task: 't1',
          actions: [
            TriageActionDto(
              kind: 'link',
              pageId: 'p1',
              pageTitle: 'Garden plan',
              target: 'p2',
              targetTitle: 'Seed list',
            ),
            TriageActionDto(
              kind: 'tag',
              pageId: 'p2',
              pageTitle: 'Seed list',
              target: 'garden',
              targetTitle: '',
            ),
          ],
        ),
      ],
    );
    await _panel(tester, fake);

    expect(find.text("Linked 'Garden plan' to 'Seed list'"), findsOneWidget);
    expect(find.text("Tagged 'Seed list' with garden"), findsOneWidget);
    expect(find.text('Links and tags added by Triage'), findsOneWidget);

    await tester.tap(_k('applied-undo-t1'));
    await tester.pumpAndSettle();

    expect(fake.calls, ['undo-applied:t1']);
    expect(_k('applied-t1'), findsNothing);
    expect(
      find.text('No proposals. Run Triage on a project to look for duplicates.'),
      findsOneWidget,
    );
  });

  testWidgets('Dismiss calls dismiss and removes the entry', (tester) async {
    final fake = FakeTriage([_p('a'), _p('b', keep: 'Trip', dropped: 'Trip 2')]);
    await _panel(tester, fake);

    await tester.tap(_k('proposal-dismiss-a'));
    await tester.pumpAndSettle();

    expect(fake.calls, ['dismiss:a']);
    expect(_k('proposal-a'), findsNothing);
    expect(_k('proposal-b'), findsOneWidget);
  });

  testWidgets('empty state names the next step', (tester) async {
    await _panel(tester, FakeTriage([]));
    expect(
      find.text('No proposals. Run Triage on a project to look for duplicates.'),
      findsOneWidget,
    );
  });

  testWidgets('a title opens its page', (tester) async {
    final opened = <String>[];
    await _panel(tester, FakeTriage([_p('a')]), opened: opened, mobile: true);

    await tester.tap(_k('proposal-keep-a'));
    await tester.tap(_k('proposal-drop-a'));
    expect(opened, ['k-a:Garden plan', 'd-a:Garden plan copy']);
  });

  testWidgets('the store reloads when the proposals page changes', (
    tester,
  ) async {
    final fake = FakeTriage([_p('a')]);
    final store = await _panel(tester, fake);
    expect(store.pending, 1);

    fake.items = [_p('a'), _p('b', keep: 'Trip', dropped: 'Trip 2')];
    ChangeBatch batch(String page) => ChangeBatch(
      seq: 1,
      reset: false,
      changes: [Change(kind: ChangeKind.pageChanged, page: page)],
    );
    store.onBatch(batch('other'));
    await tester.pumpAndSettle();
    expect(store.pending, 1, reason: 'a change elsewhere is ignored');

    store.onBatch(batch('nodes'));
    await tester.pumpAndSettle();
    expect(store.pending, 2);
    expect(_k('proposal-b'), findsOneWidget);
  });

  for (final mobile in [false, true]) {
    testWidgets(
      'Triage on a project starts the run on the picked runner (${mobile ? 'phone' : 'desktop'})',
      (tester) async {
        final fake = FakeTriage([]);
        await _mount(
          tester,
          (context) => Center(
            child: GestureDetector(
              key: const ValueKey('go'),
              onTap: () => startTriageWith(
                context,
                bridge: fake,
                runners: FakeRunners(),
                hubUsable: true,
                projectId: 'proj',
                projectTitle: 'Garden',
              ),
              child: const Text('go'),
            ),
          ),
          mobile: mobile,
        );

        await tester.tap(_k('go'));
        await tester.pumpAndSettle();
        expect(find.textContaining("Triage reads the pages in 'Garden'"), findsOneWidget);

        await tester.tap(_k('triage-runner-laptop'));
        await tester.pumpAndSettle();

        expect(fake.calls, ['start:proj:laptop']);
        expect(
          find.text('Triage started on Laptop. Proposals appear under Merge proposals.'),
          findsOneWidget,
        );
        await tester.pump(const Duration(seconds: 5));
      },
    );
  }
}
