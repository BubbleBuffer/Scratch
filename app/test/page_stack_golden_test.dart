@Tags(['golden'])
library;

import 'dart:async';

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/editor/page_screen.dart';
import 'package:notes_app/shell/home.dart';
import 'package:notes_app/shell/navigation.dart';

import 'harness.dart';

PageEditorState topEditor(WidgetTester t) {
  final stack = PageStackScope.of(t.element(find.byType(HomeScreen)));
  return t.state<PageEditorState>(
    find.byWidgetPredicate((w) => w is PageScreen && w.pageId == stack.topId),
  );
}

Future<void> openSheet(WidgetTester t, String id, String title) async {
  final stack = PageStackScope.of(t.element(find.byType(HomeScreen)));
  final context = stack.pages.isEmpty
      ? t.element(find.byType(HomeScreen))
      : topEditor(t).context;
  unawaited(openPage(context, id, title, stack: true));
  await settle(t);
}

Future<void> shot(String name) => expectLater(
  find.byType(Directionality).first,
  matchesGoldenFile('../screenshots/$name.png'),
);

void main() {
  setUpAll(initCore);
  for (final dark in [false, true]) {
    for (final count in [2, 3]) {
      appTest('$count desktop sheets ${dark ? 'dark' : 'light'}', (t) async {
        final s = await pumpApp(t, mobile: false, dark: dark);
        await openSheet(t, s.showcaseId, 'Bake-off showcase');
        await openSheet(t, s.pages['Recipes']!, 'Recipes');
        if (count == 3) await openSheet(t, s.pages['Budget']!, 'Budget');
        await shot('desktop-${dark ? 'dark' : 'light'}-stack-$count');
      });
    }
  }
  appTest('desktop sheet push midway light', (t) async {
    final s = await pumpApp(t, mobile: false);
    await openSheet(t, s.showcaseId, 'Bake-off showcase');
    final id = s.pages['Recipes']!;
    unawaited(openPage(topEditor(t).context, id, 'Recipes', stack: true));
    await t.pump();
    // Let the real core load without advancing the route's fake animation.
    for (var attempt = 0; attempt < 30 && topEditor(t).doc == null; attempt++) {
      await t.runAsync(
        () => Future<void>.delayed(const Duration(milliseconds: 5)),
      );
      await t.pump();
    }
    expect(topEditor(t).doc, isNotNull);
    await t.pump(const Duration(milliseconds: 125));
    await shot('desktop-light-stack-transition-midway');
  });
}
