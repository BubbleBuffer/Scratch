// The database add-on's public API. Other add-ons and the app's remaining
// code import only this file; everything else in this folder is private.

import 'package:flutter/widgets.dart';

import '../../registry/addon_registry.dart';
import '../../registry/block_codecs.dart';
import '../../registry/command_registry.dart';
import '../../registry/note_type_registry.dart';
import '../../ui/note_preview.dart';
import 'blocks/database.dart';
import 'create.dart';
import 'database_view.dart';
import 'properties/properties.dart';
import 'views/views.dart';

export 'blocks/database.dart' show databaseBlock, databaseKind;
export 'controller.dart' show DbController;
export 'create.dart' show makeDatabase;
export 'editors/date_picker.dart' show DatePicker;
export 'page_properties.dart' show PageProperties;
export 'property_menu.dart' show propIcon;
export 'properties/properties.dart' show registerBuiltinProperties;
export 'views/common.dart';
export 'views/card.dart';
export 'views/dates.dart';
export 'views/views.dart' show registerBuiltinViews;

class DatabaseAddon extends Addon {
  const DatabaseAddon();
  @override
  String get id => 'database';
  @override
  List<String> get dependsOn => const [];
  @override
  void register(AddonContext ctx) {
    ctx.registerNoteType(
      NoteType(
        id: 'database',
        label: 'Database',
        chip: noteChip,
        card: noteCard,
        embed: _embed,
        fullPage: _fullPage,
        usesEditor: false,
        changes: madeDatabases,
        claimsPage: (id) => madeDatabases.value.contains(id),
        commands: [
          Command(
            id: 'database.make',
            label: 'Turn into database',
            isEnabled: (context) =>
                ctx.services.selectedPageId(context) != null,
            run: (context) {
              final id = ctx.services.selectedPageId(context);
              if (id != null) makeDatabase(ctx.services.vault(context), id);
            },
          ),
        ],
      ),
    );
    ctx.blockKinds.register(
      databaseBlock.inGroup(databaseBlock.group, codecs: genericBlockCodecs),
      before: 'callout',
    );
    registerBuiltinProperties(ctx.properties);
    registerBuiltinViews(ctx.views);
  }
}

Widget _fullPage(BuildContext context, NoteArgs args) => DatabaseView(
  key: ValueKey('db-page-${args.pageId}'),
  collection: args.pageId,
  title: args.titleController,
  onTitleChanged: args.onTitleChanged,
);

Widget _embed(BuildContext context, NoteArgs args) => LayoutBuilder(
  builder: (context, box) => DatabaseView(
    key: ValueKey('inline-db-${args.pageId}'),
    collection: args.pageId,
    inline: true,
    width: box.maxWidth,
  ),
);
