// The chip's results panel (Form B), opened from a test host: a popover on
// desktop that closes on Escape and on "Open in Search", and a bottom sheet on
// phone whose Undo message sits above the footer, never over its buttons.

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/addons/search_card/query_panel.dart';
import 'package:notes_app/addons/search_card/query_watch.dart';
import 'package:notes_app/state/app_state.dart';

import 'search_card_fixture.dart';

/// Opens a panel for [query] and waits for its first result. Returns the
/// watch, which the test disposes.
Future<QueryWatch> _openPanel(
  WidgetTester tester, {
  required String query,
  required void Function() onOpenInSearch,
  void Function()? onShowAsBlock,
}) async {
  final context = tester.element(find.byType(Overlay).first);
  final watch = QueryWatch(
    vault: AppScope.of(context).vault,
    tasksOn: true,
    query: query,
  )..start();
  showQueryPanel(
    context,
    watch: watch,
    anchor: const Rect.fromLTWH(600, 200, 40, 30),
    onOpenInSearch: onOpenInSearch,
    onShowAsBlock: onShowAsBlock ?? () {},
  );
  await settleUntil(tester, () => watch.ready);
  await settle(tester);
  return watch;
}

void main() {
  setUpAll(initCore);

  appTest('the desktop panel lists the matching pages and closes on Escape', (
    tester,
  ) async {
    await pumpApp(tester, mobile: false);
    await seedPage(tester, 'Quarterly review');
    await seedPage(tester, 'Quarterly budget');
    final watch = await _openPanel(
      tester,
      query: 'quarterly',
      onOpenInSearch: () {},
    );

    final panel = find.byKey(const ValueKey('query-panel'));
    expect(panel, findsOneWidget);
    expect(
      find.descendant(of: panel, matching: find.text('Quarterly review')),
      findsOneWidget,
    );
    expect(
      find.descendant(of: panel, matching: find.text('Quarterly budget')),
      findsOneWidget,
    );
    expect(
      find.descendant(
        of: panel,
        matching: find.byKey(const ValueKey('query-panel-open-search')),
      ),
      findsOneWidget,
    );
    expect(tester.takeException(), isNull);

    await tester.sendKeyEvent(LogicalKeyboardKey.escape);
    await settle(tester);
    expect(panel, findsNothing);
    watch.dispose();
  });

  appTest('"Open in Search" closes the desktop panel and runs its callback', (
    tester,
  ) async {
    await pumpApp(tester, mobile: false);
    await seedPage(tester, 'Quarterly review');
    var opened = 0;
    final watch = await _openPanel(
      tester,
      query: 'quarterly',
      onOpenInSearch: () => opened++,
    );

    await tapAndSettle(
      tester,
      find.byKey(const ValueKey('query-panel-open-search')),
    );
    expect(opened, 1);
    expect(find.byKey(const ValueKey('query-panel')), findsNothing);
    watch.dispose();
  });

  appTest('a phone Undo message sits above the footer, not over its buttons', (
    tester,
  ) async {
    await pumpApp(tester, mobile: true);
    final task = await seedPage(tester, 'Quarterly task', pageType: taskPageType);
    await makeTask(tester, task);
    final watch = await _openPanel(
      tester,
      query: 'quarterly',
      onOpenInSearch: () {},
    );
    expect(find.byKey(const ValueKey('query-panel-message')), findsNothing);

    // Tick the row, then report it as settled out of the list.
    final hit = watch.hits.single;
    await tapAndSettle(tester, find.byKey(ValueKey('query-check-${hit.id}')));
    expect(await taskStatus(tester, task), 'done');
    watch.reportLeftForTest(hit);
    await settle(tester);

    final message = find.byKey(const ValueKey('query-panel-message'));
    expect(message, findsOneWidget);
    final messageRect = tester.getRect(message);
    final openRect = tester.getRect(
      find.byKey(const ValueKey('query-panel-open-search')),
    );
    final asBlockRect = tester.getRect(
      find.byKey(const ValueKey('query-panel-as-block')),
    );
    expect(messageRect.bottom, lessThanOrEqualTo(openRect.top));
    expect(messageRect.bottom, lessThanOrEqualTo(asBlockRect.top));
    expect(tester.takeException(), isNull);

    // Undo writes the task back to not done and clears the message.
    await tapAndSettle(tester, find.byKey(const ValueKey('query-panel-undo')));
    expect(message, findsNothing);
    expect(await taskStatus(tester, task), 'todo');
    watch.dispose();
  });
}
