import 'dart:io';

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/core/models.dart';
import 'package:notes_app/features/settings/settings_store.dart';
import 'package:notes_app/registry/addon_registry.dart';
import 'package:notes_app/registry/block_registry.dart';
import 'package:notes_app/registry/command_registry.dart';
import 'package:notes_app/registry/note_type_registry.dart';
import 'package:notes_app/registry/property_registry.dart';
import 'package:notes_app/registry/view_registry.dart';
import 'package:notes_app/shell/addons.dart';

List<Addon> closure(Addon addon) {
  final byId = {for (final entry in addonList) entry.addon.id: entry.addon};
  final needed = <String, Addon>{};
  void add(Addon value) {
    if (needed.containsKey(value.id)) return;
    needed[value.id] = value;
    for (final id in value.dependsOn) {
      add(byId[id]!);
    }
  }

  add(addon);
  return needed.values.toList();
}

void main() {
  for (final entry in addonList) {
    test(
      '${entry.addon.id} boots with only its dependencies, without native initialization',
      () {
        final globalBlocks = blockRegistry.all.toList();
        final globalTypes = noteTypeRegistry.all.toList();
        final globalViews = viewRegistry.all.toList();
        final globalProperties = propertyRegistry.all.toList();
        final globalCommands = commandRegistry.all.toList();
        final ctx = AddonContext();
        final needed = closure(entry.addon);
        final booted = bootAddons(needed, ctx);
        expect(
          booted.map((a) => a.id).toSet(),
          needed.map((a) => a.id).toSet(),
        );
        expect(blockRegistry.all, globalBlocks);
        expect(noteTypeRegistry.all, globalTypes);
        expect(viewRegistry.all, globalViews);
        expect(propertyRegistry.all, globalProperties);
        expect(commandRegistry.all, globalCommands);
        if (entry.addon.id == 'board') {
          expect(ctx.views.of(ViewKind.board), isNotNull);
          expect(ctx.noteTypes.of('database'), isNotNull);
          expect(ctx.noteTypes.of('document'), isNull);
          expect(ctx.blockKinds.has(Kinds.paragraph), isFalse);
        }
        ctx.noteTypes.dispose();
      },
    );
  }

  test(
    'every registered block has all codecs, including insert-only entries',
    () {
      final ctx = AddonContext();
      bootAddons(addonList.map((entry) => entry.addon), ctx);
      expect(ctx.blockKinds.all, isNotEmpty);
      expect(ctx.blockKinds.incompleteCodecKinds, isEmpty);
      for (final type in ctx.blockKinds.all) {
        expect(
          type.codecs,
          isNotNull,
          reason: '${type.kind} must declare its codecs',
        );
      }
      ctx.noteTypes.dispose();
    },
  );

  test('default menus retain the existing block and view order', () {
    final ctx = AddonContext();
    bootAddons(addonList.map((entry) => entry.addon), ctx);
    final menu = ctx.blockKinds.menuTypes.map((type) => type.kind).toList();
    expect(menu.indexOf('database'), menu.indexOf(Kinds.numbered) + 1);
    expect(menu.indexOf(Kinds.callout), menu.indexOf('database') + 1);
    expect(ctx.views.all.map((type) => type.kind), [
      ViewKind.table,
      ViewKind.board,
      ViewKind.timeline,
      ViewKind.calendar,
      ViewKind.list,
      ViewKind.gallery,
    ]);
    ctx.noteTypes.dispose();
  });

  test('optional add-ons disable their registrations; required entries stay enabled', () {
    final ctx = AddonContext();
    final selected = enabledAddons({
      'agent',
      'board',
      'database',
      'document',
      'kanban',
      'schedule',
      'tasks',
      'legacy',
      'stale',
    });
    bootAddons(selected, ctx);
    expect(selected.map((a) => a.id), [
      'legacy',
      'document',
      'database',
      'search_card',
    ]);
    expect(ctx.noteTypes.of('task'), isNull);
    expect(ctx.shellNavigation.all, isEmpty);
    expect(ctx.shellPrimaryActions.all, isEmpty);
    expect(ctx.titleChips.all, isEmpty);
    expect(ctx.commands.of('calendar.open'), isNull);
    expect(ctx.noteTypes.of('kanban'), isNull);
    expect(enabledAddons({}).map((a) => a.id), [
      'legacy',
      'agent',
      'document',
      'database',
      'board',
      'kanban',
      'schedule',
      'tasks',
      'search_card',
    ]);
    expect(ctx.views.of(ViewKind.board), isNull);
    expect(ctx.views.of(ViewKind.table), isNotNull);
    expect(ctx.noteTypes.of('database'), isNotNull);
    expect(ctx.blockKinds.has(Kinds.paragraph), isTrue);
    ctx.noteTypes.dispose();
  });

  test('optional choices persist in device settings', () {
    final dir = Directory.systemTemp.createTempSync('notes-addon-settings-');
    addTearDown(() => dir.deleteSync(recursive: true));
    final file = File('${dir.path}/settings.json');
    final settings = AppSettings(file: file);
    settings.update((s) => s.disabledAddons.add('board'));
    expect(AppSettings.load(file).disabledAddons, {'board'});
    settings.update((s) => s.disabledAddons.remove('board'));
    expect(AppSettings.load(file).disabledAddons, isEmpty);
    settings.dispose();
  });

  testWidgets(
    'host draws a custom note type and preserves unknown markdown fallback',
    (tester) async {
      final registry = NoteTypeRegistry();
      addTearDown(registry.dispose);
      Widget builder(BuildContext context, NoteArgs args) =>
          const Text('Custom type body');
      registry.register(
        NoteType(
          id: 'custom',
          label: 'Custom',
          chip: builder,
          card: builder,
          embed: builder,
          fullPage: builder,
        ),
      );
      Future<void> show(String type) => tester.pumpWidget(
        Directionality(
          textDirection: TextDirection.ltr,
          child: NotePageBody(
            registry: registry,
            args: NoteArgs(
              pageId: 'a',
              pageType: type,
              editor: const Text('Markdown fallback'),
            ),
          ),
        ),
      );
      await show('custom');
      expect(find.text('Custom type body'), findsOneWidget);
      expect(find.text('Markdown fallback'), findsNothing);
      await show('not-installed');
      expect(find.text('Markdown fallback'), findsOneWidget);
    },
  );

  testWidgets('optimistic type changes redraw through the registry', (
    tester,
  ) async {
    final registry = NoteTypeRegistry();
    final ids = ValueNotifier<Set<String>>({});
    addTearDown(registry.dispose);
    addTearDown(ids.dispose);
    Widget builder(BuildContext context, NoteArgs args) =>
        const Text('Changed body');
    registry.register(
      NoteType(
        id: 'changed',
        label: 'Changed',
        chip: builder,
        card: builder,
        embed: builder,
        fullPage: builder,
        claimsPage: (id) => ids.value.contains(id),
        changes: ids,
      ),
    );
    await tester.pumpWidget(
      Directionality(
        textDirection: TextDirection.ltr,
        child: NotePageBody(
          registry: registry,
          args: const NoteArgs(pageId: 'a', editor: Text('Original body')),
        ),
      ),
    );
    expect(find.text('Original body'), findsOneWidget);
    ids.value = {'a'};
    await tester.pump();
    expect(find.text('Changed body'), findsOneWidget);
  });
}
