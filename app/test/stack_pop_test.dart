// Popping a row sheet back to its database with the spine, then opening the
// row again or editing its title cell, leaves the pile healthy. Also
// push/pop/push cycles at depths 1 to 4. (The 2026-10-07 replay crash on
// this path came from the drive host's `state` command, not the app.)

import 'dart:async';

import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/shell/home.dart';
import 'package:notes_app/shell/navigation.dart' as nav;

import 'db_harness.dart';
import 'stack_flow.dart';

/// A new page made a database, with one new row open on top whose title is
/// "Day 1". Returns (database, row).
Future<(String, String)> _databaseWithRow(WidgetTester tester) async {
  unawaited(nav.createPage(tester.element(find.byType(HomeScreen))));
  await settle(tester);
  final page = stackOf(tester).topId!;
  await tester.enterText(
    find.descendant(
      of: byKey('stack-sheet-$page'),
      matching: byKey('page-title'),
    ),
    'Lisbon trip',
  );
  await settle(tester);
  await tapKey(tester, 'start-database');
  await settleUntil(tester, () => byKey('db-new').evaluate().isNotEmpty);
  await tapKey(tester, 'db-new');
  final row = stackOf(tester).topId!;
  expect(row, isNot(page));
  final title = find.descendant(
    of: byKey('stack-sheet-$row'),
    matching: byKey('page-title'),
  );
  await tester.tap(title);
  await settle(tester);
  await tester.enterText(title, 'Day 1');
  await settle(tester, min: const Duration(milliseconds: 600));
  // Back to the database with a real tap on the spine.
  await tester.tap(byKey('stack-spine-$page'));
  await settle(tester);
  expect(stackOf(tester).pages.map(stackOf(tester).idOf), [page]);
  return (page, row);
}

void main() {
  setUpAll(initCore);


  dbTest('after popping a row with the spine, OPEN opens it again', (
    tester,
  ) async {
    await pumpDb(tester, mobile: false);
    final (page, row) = await _databaseWithRow(tester);
    await settleUntil(tester, () => byKey('title-Day 1').evaluate().isNotEmpty);
    final g = await hover(tester, byKey('title-Day 1'));
    await g.removePointer();
    await tapKey(tester, 'open-Day 1');
    expect(tester.takeException(), isNull);
    expect(stackOf(tester).pages.map(stackOf(tester).idOf), [page, row]);
  });

  dbTest('after popping a row with the spine, editing its cell is safe', (
    tester,
  ) async {
    await pumpDb(tester, mobile: false);
    final (page, _) = await _databaseWithRow(tester);
    await settleUntil(tester, () => byKey('title-Day 1').evaluate().isNotEmpty);
    await tester.tap(byKey('title-Day 1'));
    await settle(tester);
    await tester.sendKeyEvent(LogicalKeyboardKey.escape);
    await settle(tester);
    expect(tester.takeException(), isNull);
    expect(stackOf(tester).pages.map(stackOf(tester).idOf), [page]);
  });

  dbTest('push, pop and push again at depths 1 to 4', (tester) async {
    await pumpDb(tester, mobile: false);
    final ids = <String>[];
    for (final t in ['Weekly review', 'Garden plan', 'Reading notes']) {
      final pages = await core(tester, (v) => v.listChildren(null));
      ids.add(pages.firstWhere((p) => p.title == t).id);
    }
    ids.add(db.host);
    Future<void> push(int i) async {
      final s = stackOf(tester);
      final ctx = s.pages.isEmpty
          ? tester.element(find.byType(HomeScreen))
          : editorOf(tester, s.topId!).context;
      unawaited(nav.openPage(ctx, ids[i], '', stack: true));
      await settle(tester);
    }

    for (var depth = 1; depth <= 4; depth++) {
      for (var i = 0; i < depth; i++) {
        await push(i);
      }
      for (var round = 0; round < 2; round++) {
        if (depth > 1) {
          // Pop with the spine, then push the same page again.
          await tester.tap(byKey('stack-spine-${ids[depth - 2]}'));
          await settle(tester);
          expect(stackOf(tester).pages.length, depth - 1);
          await push(depth - 1);
        } else {
          await tester.sendKeyEvent(LogicalKeyboardKey.escape);
          await settle(tester);
          expect(stackOf(tester).pages, isEmpty);
          await push(0);
        }
        expect(tester.takeException(), isNull);
        expect(stackOf(tester).pages.length, depth);
        expectOnTop(tester, ids.take(depth).toList());
      }
      stackOf(tester).clearImmediately();
      await settle(tester);
    }
  });
}
