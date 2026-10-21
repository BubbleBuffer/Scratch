import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/shell/search.dart';

import 'harness.dart';

const hint = 'Showing title matches — keep typing to search page text';

void main() {
  setUpAll(initCore);

  appTest('Ctrl+K hints for broad queries and clears for selective input', (
    tester,
  ) async {
    final fixture = await pumpApp(tester, mobile: false, empty: true);
    final vault = fixture.app.vault;
    final garden = (await tester.runAsync(() async {
      // More than the 200-document floor, with a selective longer prefix.
      for (var i = 0; i < 201; i++) {
        await vault.createPage(title: 'Growth $i');
      }
      final garden = await vault.createPage(title: 'Garden');
      await vault.flush();
      return garden;
    }))!;
    await settle(tester);
    await tester.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
    await tester.sendKeyEvent(LogicalKeyboardKey.keyK);
    await tester.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);
    await settle(tester);
    expect(find.byKey(const ValueKey('search-overlay')), findsOneWidget);
    final field = find.descendant(
      of: find.byKey(const ValueKey('search-overlay-field')),
      matching: find.byType(EditableText),
    );
    await tester.enterText(field, 'g');
    await settle(tester);
    expect(find.text(hint), findsOneWidget);
    expect(
      tester.widgetList<SearchResultRow>(find.byType(SearchResultRow)),
      isNotEmpty,
    );
    expect(
      tester
          .widgetList<SearchResultRow>(find.byType(SearchResultRow))
          .every((row) => row.hit.snippet.isEmpty),
      isTrue,
    );
    await tester.enterText(field, 'garden');
    await settle(tester);
    expect(find.text(hint), findsNothing);
    expect(find.byKey(ValueKey('hit-$garden')), findsOneWidget);
    await tester.enterText(field, 'g');
    await settle(tester);
    expect(find.text(hint), findsOneWidget);
    await tester.enterText(field, '');
    await settle(tester);
    expect(find.text(hint), findsNothing);
  });
}
