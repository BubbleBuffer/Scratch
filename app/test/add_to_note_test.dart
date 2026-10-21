// "Add to note" for a chat reference, on the real vault: recent notes list,
// search narrows it, the reference is appended as the note's last top-level
// block with its target payload, a notice offers Open, and "New note" makes
// a page first.

import 'dart:async';
import 'dart:convert';

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/core/models.dart';
import 'package:notes_app/addons/agent/api.dart';

import 'chat_reference_fixture.dart';
import 'harness.dart';
import 'sync_helpers.dart';

Future<void> _open(WidgetTester tester, ChatReference ref) async {
  final ctx = tester.element(find.byType(Overlay).first);
  unawaited(showAddChatReference(ctx, ref));
  await settle(tester);
}

void main() {
  setUpAll(initCore);

  final ref = ChatReference.fromChapter(
    sessionId: 'sess-1',
    chatTitle: chatRefChatTitle,
    chapter: chatRefChapter(),
  );

  for (final mobile in [false, true]) {
    appTest('Add to note fits a small ${mobile ? 'phone' : 'desktop'}', (
      tester,
    ) async {
      tester.view.physicalSize = Size(mobile ? 390 : 650, 600);
      tester.view.devicePixelRatio = 1;
      addTearDown(tester.view.resetPhysicalSize);
      addTearDown(tester.view.resetDevicePixelRatio);
      await pumpApp(tester, mobile: mobile);
      await _open(tester, ref);
      expect(find.byKey(const ValueKey('add-show-card')), findsOneWidget);
      expect(find.byKey(const ValueKey('add-show-link')), findsOneWidget);
      expect(tester.takeException(), isNull);
    });
  }

  appTest(
    'the recent notes list offers the reference target and the search narrows it',
    (tester) async {
      final tv = await pumpApp(tester, mobile: false);
      final recipes = tv.pages['Recipes']!;
      final soups = tv.pages['Soups']!;
      // Make the intended target recent, independently of fixture seed order.
      await core(tester, (v) => v.renamePage(recipes, 'Edited recipes'));
      await real(tester, tv.app.pages.reload);
      await _open(tester, ref);
      expect(find.byKey(ValueKey('add-note-$recipes')), findsOneWidget);
      expect(find.byKey(const ValueKey('add-note-new')), findsOneWidget);

      await tester.enterText(find.byKey(const ValueKey('add-search')), 'Soups');
      await settleUntil(
        tester,
        () => find.byKey(ValueKey('add-note-$soups')).evaluate().isNotEmpty,
      );
      expect(find.byKey(ValueKey('add-note-$soups')), findsOneWidget);
      expect(
        find.byKey(ValueKey('add-note-$recipes')),
        findsNothing,
        reason: 'search replaces the recent list',
      );
    },
  );

  appTest('add appends the reference to the chosen note and offers Open', (
    tester,
  ) async {
    final tv = await pumpApp(tester, mobile: false);
    await _open(tester, ref);

    final soups = tv.pages['Soups']!;
    await tester.enterText(find.byKey(const ValueKey('add-search')), 'Soups');
    await settleUntil(
      tester,
      () => find.byKey(ValueKey('add-note-$soups')).evaluate().isNotEmpty,
    );
    await tester.tap(find.byKey(ValueKey('add-note-$soups')));
    await settle(tester);
    await tester.tap(find.byKey(const ValueKey('add-go')));
    await settleUntil(
      tester,
      () => find.byKey(const ValueKey('add-notice')).evaluate().isNotEmpty,
    );

    expect(find.byKey(const ValueKey('add-notice')), findsOneWidget);
    expect(find.byKey(const ValueKey('add-notice-open')), findsOneWidget);

    final page = await core(tester, (v) => v.loadPage(soups));
    final last = page.blocks.lastWhere((b) => b.depth == 0);
    expect(last.kind, Kinds.chatRef);
    expect(
      last.text,
      ref.searchText,
      reason: 'the summary is the searchable text',
    );
    final target = jsonDecode(last.target!) as Map<String, Object?>;
    expect(target['sessionId'], 'sess-1');
    expect(target['firstMessageId'], 'm-40');
    expect(target['lastMessageId'], 'm-88');
    expect(ChatReference.fromTarget(last.target)!.label, ref.label);
  });

  appTest('the summary of the added reference is found by notes search', (
    tester,
  ) async {
    final tv = await pumpApp(tester, mobile: false);
    await _open(tester, ref);

    final soups = tv.pages['Soups']!;
    await tester.enterText(find.byKey(const ValueKey('add-search')), 'Soups');
    await settleUntil(
      tester,
      () => find.byKey(ValueKey('add-note-$soups')).evaluate().isNotEmpty,
    );
    await tester.tap(find.byKey(ValueKey('add-note-$soups')));
    await settle(tester);
    await tester.tap(find.byKey(const ValueKey('add-go')));
    await settleUntil(
      tester,
      () => find.byKey(const ValueKey('add-notice')).evaluate().isNotEmpty,
    );

    final hits = await core(tester, (v) => v.search('deployed', limit: 10));
    expect(hits.map((h) => h.id), contains(soups));
  });

  appTest('New note creates a page and appends the reference to it', (
    tester,
  ) async {
    final tv = await pumpApp(tester, mobile: false);
    final before = await core(tester, (v) => v.pageCount());
    await _open(tester, ref);

    await tester.ensureVisible(find.byKey(const ValueKey('add-note-new')));
    await tester.tap(find.byKey(const ValueKey('add-note-new')));
    await settle(tester);
    await tester.tap(find.byKey(const ValueKey('add-go')));
    await settleUntil(
      tester,
      () => find.byKey(const ValueKey('add-notice')).evaluate().isNotEmpty,
    );

    final after = await core(tester, (v) => v.pageCount());
    expect(after, before + 1);
    expect(
      tv.pages.containsKey('Soups'),
      isTrue,
      reason: 'fixture pages are untouched',
    );
  });

  appTest(
    'an untitled page shows its first body line in the recent and search lists',
    (tester) async {
      final tv = await pumpApp(tester, mobile: false);
      final untitled = await core(tester, (v) async {
        final id = await v.createPage();
        await v.apply(id, [
          BOp(
            op: OpKind.insert,
            id: newId(),
            kind: Kinds.paragraph,
            text: 'Groceries for Sunday',
            marks: const [],
            checked: false,
          ),
        ]);
        return id;
      });
      await real(tester, tv.app.pages.reload);
      await _open(tester, ref);

      final row = find.byKey(ValueKey('add-note-$untitled'));
      await settleUntil(
        tester,
        () => find
            .descendant(of: row, matching: find.text('Groceries for Sunday'))
            .evaluate()
            .isNotEmpty,
      );
      expect(
        find.descendant(of: row, matching: find.text('Untitled')),
        findsNothing,
      );

      // A no-hit search first: the row leaving proves the search ran.
      final search = find.byKey(const ValueKey('add-search'));
      await tester.enterText(search, 'zzz no such note');
      await settleUntil(tester, () => row.evaluate().isEmpty);

      await tester.enterText(search, 'Groceries');
      await settleUntil(tester, () => row.evaluate().isNotEmpty);
      expect(
        find.descendant(of: row, matching: find.text('Groceries for Sunday')),
        findsOneWidget,
      );
      expect(
        find.descendant(of: row, matching: find.text('Untitled')),
        findsNothing,
      );
      await tester.tap(row);
      await settle(tester);
      await tester.tap(find.byKey(const ValueKey('add-go')));
      await settleUntil(
        tester,
        () => find.byKey(const ValueKey('add-notice')).evaluate().isNotEmpty,
      );
      expect(find.text('Added to Groceries for Sunday'), findsOneWidget);
    },
  );
}
