// Registered search sources with no add-on behind them: the app boots with the
// agent add-on disabled, and fake sources (plain AddonSearchSource subclasses,
// not the chat source) are registered into the app's search registry. Ctrl+K
// groups a source's rows under its own group and type chip, with no Chats
// group, and opens them with the source's opaque target. A cached agent row in
// the vault index is excluded because no registered source owns it. The
// indexed path is covered with a fake indexed source whose rows live in the
// vault index.

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/registry/addon_registry.dart';
import 'package:notes_app/shell/search.dart';
import 'package:notes_app/src/rust/api/types.dart';

import '../addon_harness.dart';
import '../sync_helpers.dart';

/// A non-indexed source: its query callback supplies the rows, and its target
/// is a string the host must hand back unchanged.
class FakeKitSource extends AddonSearchSource {
  FakeKitSource();

  static const target = 'kit:opaque/k-1#7';

  final List<String> queries = [];
  final List<AddonSearchResult> opened = [];

  void reset() {
    queries.clear();
    opened.clear();
  }

  @override
  String get id => 'kit';
  @override
  String get label => 'Kit';
  @override
  String get groupId => 'kit-group';
  @override
  Set<String> get noteTypes => const {'kit'};
  @override
  String get emptyTitle => 'Untitled kit';

  @override
  Future<List<AddonSearchResult>> search(String query) async {
    queries.add(query);
    return const [
      AddonSearchResult(
        id: 'k-1',
        title: 'Lantern kit row',
        description: 'Kit body text.',
        pageType: 'kit',
        target: target,
      ),
    ];
  }

  @override
  Future<void> open(BuildContext context, AddonSearchResult result) async {
    opened.add(result);
  }
}

/// An indexed source: its rows are the vault index's, so its query callback is
/// never called. [live] turns the indexed flag on for one test only, so the
/// other tests keep the plain page-search path.
class FakeIndexedSource extends AddonSearchSource {
  FakeIndexedSource();

  static const target = 'ix:opaque/ix-1';

  bool live = false;
  final List<AddonSearchResult> opened = [];

  void reset() {
    live = false;
    opened.clear();
  }

  @override
  String get id => 'fake-index';
  @override
  String get label => 'Indexed kit';
  @override
  String get groupId => 'indexed-group';
  @override
  bool get indexed => live;

  @override
  Future<void> open(BuildContext context, AddonSearchResult result) async {
    opened.add(result);
  }
}

final kit = FakeKitSource();
final indexed = FakeIndexedSource();

/// The EditableText inside the overlay's search field.
Finder _overlayEditable() => find.descendant(
  of: find.byKey(const ValueKey('search-overlay-field')),
  matching: find.byType(EditableText),
);

/// Opens the desktop overlay with Ctrl+K and types [query] into it.
Future<void> _searchFor(WidgetTester tester, String query) async {
  await tester.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
  await tester.sendKeyEvent(LogicalKeyboardKey.keyK);
  await tester.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);
  await settle(tester);
  expect(find.byKey(const ValueKey('search-overlay')), findsOneWidget);
  await tester.enterText(_overlayEditable(), query);
  await settle(tester);
}

/// Writes one row into the vault index under [source], flushed before return.
Future<void> _indexRow(
  WidgetTester tester,
  TestVault tv, {
  required String source,
  required String id,
  required String title,
  required String pageType,
  String? target,
}) async {
  await real(tester, () async {
    await tv.app.vault.run(
      (v) => v.upsertSearchRow(
        source: source,
        row: ExternalSearchRow(
          id: id,
          title: title,
          body: '',
          pageType: pageType,
          target: target,
          updatedMs: 0,
        ),
      ),
    );
    await tv.app.vault.flush();
  });
}

void main() {
  setUpAll(() async {
    registerBuiltins(disabledAddons: {'agent'});
    await initCore();
    appAddonContext.searchSources.register(kit);
    appAddonContext.searchSources.register(indexed);
  });

  setUp(() {
    kit.reset();
    indexed.reset();
  });

  appTest('boots with agent disabled and both fake sources registered', (
    tester,
  ) async {
    expect(activeAddonContext!.installedIds, isNot(contains('agent')));
    expect(appAddonContext.searchSources.of('agent'), isNull);
    expect(identical(appAddonContext.searchSources.of('kit'), kit), isTrue);
    expect(
      identical(appAddonContext.searchSources.of('fake-index'), indexed),
      isTrue,
    );
  });

  appTest(
    'Ctrl+K groups a registered source under its own group and chip, with no Chats group',
    (tester) async {
      final tv = await pumpApp(tester, mobile: false, empty: true);
      final page = await real(
        tester,
        () => tv.app.vault.createPage(title: 'Lantern page'),
      );
      await real(tester, () => tv.app.vault.flush());

      await _searchFor(tester, 'lantern');
      final row = find.byKey(const ValueKey('hit-kit/k-1'));
      await settleUntil(tester, () => row.evaluate().isNotEmpty);

      // The source's query callback ran with the typed query.
      expect(kit.queries, contains('lantern'));

      // Its group header and type chip, labelled from the source.
      expect(
        find.byKey(const ValueKey('search-group-kit-group')),
        findsOneWidget,
      );
      expect(find.text('KIT'), findsOneWidget);
      final chip = find.byKey(const ValueKey('search-type-kit-group'));
      expect(chip, findsOneWidget);
      expect(
        find.descendant(of: chip, matching: find.text('Kit')),
        findsOneWidget,
      );

      // No chat group exists when no chat source is registered.
      expect(find.byKey(const ValueKey('search-group-chats')), findsNothing);
      expect(find.text('CHATS'), findsNothing);

      // Pages come first, then the registered source's rows.
      final pageRow = find.byKey(ValueKey('hit-$page'));
      expect(pageRow, findsOneWidget);
      expect(tester.getTopLeft(pageRow).dy < tester.getTopLeft(row).dy, isTrue);

      // Choosing the chip writes its type operator into the field.
      await tester.tap(chip);
      await settleUntil(
        tester,
        () =>
            tester.widget<EditableText>(_overlayEditable()).controller.text ==
            'lantern type:kit',
      );
      expect(
        tester.widget<EditableText>(_overlayEditable()).controller.text,
        'lantern type:kit',
      );

      // Close the overlay: the app's open-once guard clears only when it closes.
      await tester.sendKeyEvent(LogicalKeyboardKey.escape);
      await settle(tester);
      expect(find.byKey(const ValueKey('search-overlay')), findsNothing);
    },
  );

  appTest('activating a source row opens it with the same opaque target', (
    tester,
  ) async {
    await pumpApp(tester, mobile: false, empty: true);
    await _searchFor(tester, 'lantern');
    final row = find.byKey(const ValueKey('hit-kit/k-1'));
    await settleUntil(tester, () => row.evaluate().isNotEmpty);

    await tester.tap(row);
    await settleUntil(tester, () => kit.opened.isNotEmpty);

    expect(kit.opened, hasLength(1));
    final opened = kit.opened.single;
    expect(opened.id, 'k-1');
    expect(opened.title, 'Lantern kit row');
    expect(opened.target, FakeKitSource.target);
    expect(find.byKey(const ValueKey('search-overlay')), findsNothing);
  });

  appTest(
    'a cached agent row is excluded while its source is unregistered; pages and the kit still show',
    (tester) async {
      final tv = await pumpApp(tester, mobile: false, empty: true);
      final page = await real(
        tester,
        () => tv.app.vault.createPage(title: 'Lantern page'),
      );
      // A chat row cached by an earlier run of the agent add-on.
      await _indexRow(
        tester,
        tv,
        source: 'agent',
        id: 'chat:cached',
        title: 'Lantern chat',
        pageType: 'chat',
        target: '{"sessionId":"cached","level":"chat"}',
      );

      // The page search keeps only the vault and the registered sources.
      final search = PageSearch(tv.app.vault, includeExternal: true);
      addTearDown(search.dispose);
      await real(tester, () => search.run('lantern'));
      expect(search.hits, isNotNull);
      expect(search.hits!.map((hit) => hit.source).toSet(), {'vault', 'kit'});
      expect(search.hits!.map((hit) => hit.id), contains(page));

      await _searchFor(tester, 'lantern');
      final row = find.byKey(const ValueKey('hit-kit/k-1'));
      await settleUntil(tester, () => row.evaluate().isNotEmpty);

      expect(find.byKey(ValueKey('hit-$page')), findsOneWidget);
      expect(find.byKey(const ValueKey('hit-agent/chat:cached')), findsNothing);
      expect(find.text('Lantern chat'), findsNothing);
      expect(find.byKey(const ValueKey('search-group-chats')), findsNothing);

      await tester.sendKeyEvent(LogicalKeyboardKey.escape);
      await settle(tester);
      expect(find.byKey(const ValueKey('search-overlay')), findsNothing);
    },
  );

  appTest(
    'the indexed path keeps a registered indexed row and still excludes the unregistered agent row',
    (tester) async {
      final tv = await pumpApp(tester, mobile: false, empty: true);
      final page = await real(
        tester,
        () => tv.app.vault.createPage(title: 'Lantern page'),
      );
      await _indexRow(
        tester,
        tv,
        source: 'agent',
        id: 'chat:cached',
        title: 'Lantern chat',
        pageType: 'chat',
        target: '{"sessionId":"cached","level":"chat"}',
      );
      await _indexRow(
        tester,
        tv,
        source: 'fake-index',
        id: 'ix-1',
        title: 'Lantern indexed row',
        pageType: 'indexed',
        target: FakeIndexedSource.target,
      );
      indexed.live = true;
      addTearDown(() => indexed.live = false);

      // With an indexed source registered, page search takes the adaptive path.
      final search = PageSearch(tv.app.vault, includeExternal: true);
      addTearDown(search.dispose);
      await real(tester, () => search.run('lantern'));
      expect(search.hits!.map((hit) => hit.source).toSet(), {
        'vault',
        'fake-index',
        'kit',
      });
      expect(search.hits!.map((hit) => hit.id), contains(page));

      await _searchFor(tester, 'lantern');
      final row = find.byKey(const ValueKey('hit-fake-index/ix-1'));
      await settleUntil(tester, () => row.evaluate().isNotEmpty);

      expect(find.byKey(ValueKey('hit-$page')), findsOneWidget);
      expect(
        find.byKey(const ValueKey('search-group-indexed-group')),
        findsOneWidget,
      );
      expect(find.text('INDEXED KIT'), findsOneWidget);
      expect(find.byKey(const ValueKey('hit-agent/chat:cached')), findsNothing);
      expect(find.byKey(const ValueKey('search-group-chats')), findsNothing);

      await tester.tap(row);
      await settleUntil(tester, () => indexed.opened.isNotEmpty);
      expect(indexed.opened, hasLength(1));
      expect(indexed.opened.single.id, 'ix-1');
      expect(indexed.opened.single.target, FakeIndexedSource.target);
    },
  );
}
