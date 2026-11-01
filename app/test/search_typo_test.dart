import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';

import 'harness.dart';
import 'search_hits_fixture.dart' show shownHits;

void main() {
  setUpAll(initCore);

  appTest('Ctrl+K explains correction only when every hit is fuzzy', (
    tester,
  ) async {
    final fixture = await pumpApp(tester, mobile: false, empty: true);
    final vault = fixture.app.vault;
    final tomato = (await tester.runAsync(() async {
      final id = await vault.createPage(title: 'Tomato');
      await vault.flush();
      return id;
    }))!;
    await settle(tester);
    await tester.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
    await tester.sendKeyEvent(LogicalKeyboardKey.keyK);
    await tester.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);
    await settle(tester);
    final field = find.descendant(
      of: find.byKey(const ValueKey('search-overlay-field')),
      matching: find.byType(EditableText),
    );
    const hintKey = ValueKey('search-correction-hint');

    await tester.enterText(field, 'tomatoe');
    await settle(tester);
    expect(find.byKey(ValueKey('hit-$tomato')), findsOneWidget);
    expect(find.byKey(hintKey), findsOneWidget);
    final hint = tester.widget<Text>(find.byKey(hintKey));
    expect(
      hint.textSpan!.toPlainText().toLowerCase(),
      'showing results for tomato',
    );
    expect(shownHits(tester).every((hit) => hit.fuzzy), isTrue);

    final exact = (await tester.runAsync(() async {
      // A substring hit leaves the query word absent from the vocabulary,
      // allowing exact-first and fuzzy results in the same response.
      final id = await vault.createPage(title: 'XtomatoeZ seedlings');
      await vault.flush();
      return id;
    }))!;
    await tester.enterText(field, '');
    await settle(tester);
    await tester.enterText(field, 'tomatoe');
    await settle(tester);
    final rows = shownHits(tester);
    expect(rows.first.id, exact);
    expect(rows.first.fuzzy, isFalse);
    expect(rows.any((hit) => hit.fuzzy), isTrue);
    expect(find.byKey(hintKey), findsNothing);

    await tester.enterText(field, 'tomato');
    await settle(tester);
    expect(find.byKey(hintKey), findsNothing);
    await tester.enterText(field, '');
    await settle(tester);
    expect(find.byKey(hintKey), findsNothing);
  });
}
