// Cards on a board: New card creates a titled document child, Add existing
// note goes through the page picker (the board itself and repeats are
// ignored), opening a card opens its note, removing a card from its context
// menu keeps the note, and a card whose note is missing stays stored but is
// hidden.

import 'dart:convert';

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/core/models.dart';
import 'package:notes_app/registry/note_type_registry.dart';

import 'kanban_fixture.dart';

void main() {
  setUpAll(initCore);

  appTest(
    'New card asks for a title and creates a titled document note under the board in that column',
    (tester) async {
      await pumpApp(tester, mobile: false);
      final board = await seedKanban(
        tester,
        title: 'Launch board',
        columns: {'To do': [], 'Doing': [], 'Done': []},
      );
      await openNote(tester, board, 'Launch board');

      // The add button opens the add panel with its title field focused.
      await tapAndSettle(tester, addButton('Doing'));
      expect(find.byKey(cardTitleFieldKey), findsOneWidget);
      await typeCardTitle(tester, 'Plan trip');
      await settleUntil(
        tester,
        () => cardIn('Doing', 'Plan trip').evaluate().isNotEmpty,
      );

      final children = await core(tester, (v) => v.listChildren(board));
      expect(children, hasLength(1));
      final note = children.single;
      expect(note.title, 'Plan trip');
      expect(note.pageType, 'document');
      expect(note.parent, board);

      final data = await storedKanban(tester, board);
      expect(storedCards(data, 'Doing'), [note.id]);
      expect(storedCards(data, 'To do'), isEmpty);
      expect(find.byKey(cardTitleFieldKey), findsNothing);
    },
  );

  appTest(
    'Add existing note uses the page picker, adds the chosen note once, and never adds the board itself',
    (tester) async {
      final f = await pumpApp(tester, mobile: false);
      final garden = f.pages['Garden plan']!;
      final board = await seedKanban(
        tester,
        title: 'Launch board',
        columns: {'To do': [], 'Doing': [], 'Done': []},
      );
      await openNote(tester, board, 'Launch board');

      Future<void> search(String text) async {
        await tester.enterText(
          find.descendant(
            of: find.byKey(const ValueKey('page-picker-field')),
            matching: find.byType(EditableText),
          ),
          text,
        );
        await settle(tester);
      }

      await tapAndSettle(tester, addButton('To do'));
      await tapAndSettle(tester, find.text(addExistingLabel));
      expect(find.byKey(const ValueKey('page-picker')), findsOneWidget);

      // The board is excluded from its own picker.
      await search('Launch board');
      expect(find.byKey(ValueKey('page-pick-$board')), findsNothing);

      await search('Garden plan');
      await tapAndSettle(tester, find.byKey(ValueKey('page-pick-$garden')));
      expect(find.byKey(const ValueKey('page-picker')), findsNothing);
      await settleUntil(
        tester,
        () => cardIn('To do', 'Garden plan').evaluate().isNotEmpty,
      );
      expect(storedCards(await storedKanban(tester, board), 'To do'), [garden]);

      // A note already on the board is ignored when picked again.
      await tapAndSettle(tester, addButton('To do'));
      await tapAndSettle(tester, find.text(addExistingLabel));
      await search('Garden plan');
      await tapAndSettle(tester, find.byKey(ValueKey('page-pick-$garden')));
      expect(find.byKey(const ValueKey('page-picker')), findsNothing);

      expect(storedCards(await storedKanban(tester, board), 'To do'), [garden]);
      expect(cardIn('To do', 'Garden plan'), findsOneWidget);
      expect(cardSurfacesIn('To do'), findsOneWidget);
    },
  );

  appTest('opening a card opens its note', (tester) async {
    final f = await pumpApp(tester, mobile: false);
    final garden = f.pages['Garden plan']!;
    final board = await seedKanban(
      tester,
      title: 'Launch board',
      columns: {
        'To do': [garden],
        'Doing': [],
        'Done': [],
      },
    );
    await openNote(tester, board, 'Launch board');
    expect(cardIn('To do', 'Garden plan'), findsOneWidget);

    await tapAndSettle(tester, cardIn('To do', 'Garden plan'));
    expect(editableWith('Some text about garden plan.'), findsOneWidget);
  });

  appTest(
    'removing a card from its context menu removes only the reference and keeps the note',
    (tester) async {
      final f = await pumpApp(tester, mobile: false);
      final garden = f.pages['Garden plan']!;
      final recipes = f.pages['Recipes']!;
      final board = await seedKanban(
        tester,
        title: 'Launch board',
        columns: {
          'To do': [garden, recipes],
          'Doing': [],
          'Done': [],
        },
      );
      await openNote(tester, board, 'Launch board');

      await openCardMenu(tester, 'Garden plan');
      expect(find.byKey(removeCardKey), findsOneWidget);
      expect(find.text(removeFromBoardLabel), findsOneWidget);
      await tapAndSettle(tester, find.byKey(removeCardKey));
      await settleUntil(
        tester,
        () => cardIn('To do', 'Garden plan').evaluate().isEmpty,
      );

      expect(storedCards(await storedKanban(tester, board), 'To do'), [
        recipes,
      ]);
      expect(cardIn('To do', 'Recipes'), findsOneWidget);

      final note = await core(tester, (v) => v.loadPage(garden));
      expect(note.title, 'Garden plan');
      final top = await core(tester, (v) => v.listChildren(null));
      expect(top.any((row) => row.id == garden), isTrue);
    },
  );

  appTest('a card whose note is missing stays in storage but is not shown', (
    tester,
  ) async {
    final f = await pumpApp(tester, mobile: false);
    final garden = f.pages['Garden plan']!;
    const ghost = 'ghost-note-that-does-not-exist';
    final board = await seedKanban(
      tester,
      title: 'Launch board',
      columns: {
        'To do': [ghost, garden],
        'Doing': [],
        'Done': [],
      },
    );
    await openNote(tester, board, 'Launch board');

    expect(cardSurfacesIn('To do'), findsOneWidget);
    expect(cardIn('To do', 'Garden plan'), findsOneWidget);
    expect(storedCards(await storedKanban(tester, board), 'To do'), [
      ghost,
      garden,
    ]);
  });

  appTest(
    'a card draws the note type registered for its page, through that type\'s card builder',
    (tester) async {
      final f = await pumpApp(tester, mobile: false);
      final garden = f.pages['Garden plan']!;
      // Unique to this test; the registry has no unregister, and no other test
      // uses this page type.
      const customType = 'kanban-test-card';
      Widget customCard(BuildContext context, NoteArgs args) => Text(
        'Custom card for ${args.title}',
        key: ValueKey('custom-card-${args.pageId}'),
      );
      // The board reads its types from the app's add-on context registry.
      appAddonContext.noteTypes.register(
        NoteType(
          id: customType,
          label: 'Custom card',
          chip: customCard,
          card: customCard,
          fullPage: customCard,
        ),
      );
      final custom = await core(
        tester,
        (v) => v.createPage(title: 'Custom note', pageType: customType),
      );
      final board = await seedKanban(
        tester,
        title: 'Launch board',
        columns: {
          'To do': [custom, garden],
          'Doing': [],
          'Done': [],
        },
      );
      await openNote(tester, board, 'Launch board');

      final cardText = find.descendant(
        of: columnNamed('To do'),
        matching: find.byKey(ValueKey('custom-card-$custom')),
      );
      expect(cardText, findsOneWidget);
      expect(tester.widget<Text>(cardText).data, 'Custom card for Custom note');
      // The fallback title is not drawn in place of the custom card.
      expect(
        find.descendant(
          of: columnNamed('To do'),
          matching: find.text('Custom note'),
        ),
        findsNothing,
      );
      // The other card still uses the default (fallback) rendering.
      expect(cardIn('To do', 'Garden plan'), findsOneWidget);

      // An optional board card takes precedence, and registry changes reach
      // an already open board without changing other note presentations.
      appAddonContext.noteTypes.register(
        NoteType(
          id: customType,
          label: 'Custom card',
          chip: customCard,
          card: customCard,
          boardCard: (context, args) => Text('Board card for ${args.title}'),
          fullPage: customCard,
        ),
      );
      await settle(tester);
      expect(find.text('Board card for Custom note'), findsOneWidget);
      expect(cardText, findsNothing);
      expect(cardIn('To do', 'Garden plan'), findsOneWidget);
    },
  );

  appTest(
    'a card whose page type is not installed shows its title and first line',
    (tester) async {
      await pumpApp(tester, mobile: false);
      const unknownType = 'kanban-test-unknown';
      final odd = await core(
        tester,
        (v) => v.createPage(title: 'Odd note', pageType: unknownType),
      );
      await core(
        tester,
        (v) => v.apply(odd, [
          const BOp(
            op: OpKind.insert,
            id: 'odd-first',
            kind: Kinds.paragraph,
            text: 'First line of the odd note.',
            marks: [],
            checked: false,
          ),
        ]),
      );
      final board = await seedKanban(
        tester,
        title: 'Launch board',
        columns: {
          'To do': [odd],
          'Doing': [],
          'Done': [],
        },
      );
      await openNote(tester, board, 'Launch board');

      expect(cardIn('To do', 'Odd note'), findsOneWidget);
      // A titled note still gets its body preview in the fallback card.
      expect(
        find.descendant(
          of: columnNamed('To do'),
          matching: find.text('First line of the odd note.'),
        ),
        findsOneWidget,
      );

      // Native writes bypass the Dart wake callback. The harness's next feed
      // poll delivers both edits together, as an incoming change batch would.
      await core(
        tester,
        (v) => v.run((raw) async {
          await raw.apply(
            pageId: odd,
            ops: [
              const BOp(
                op: OpKind.setText,
                id: 'odd-first',
                kind: Kinds.paragraph,
                text: 'Updated preview from the same batch.',
                marks: [],
                checked: false,
              ),
            ],
          );
          await raw.typeDataSet(
            page: board,
            addon: kanbanAddonId,
            path: const ['columns', 'col1', 'title'],
            jsonValue: jsonEncode('Backlog'),
          );
        }),
      );
      await settle(tester);
      expect(columnNamed('Backlog'), findsOneWidget);
      expect(
        find.descendant(
          of: columnNamed('Backlog'),
          matching: find.text('Updated preview from the same batch.'),
        ),
        findsOneWidget,
      );
    },
  );
}
