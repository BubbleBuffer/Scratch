// The database add-on's public API. Other add-ons and the app's remaining
// code import only this file; everything else in this folder is private.

import 'package:flutter/widgets.dart';

import '../../registry/addon_registry.dart';
import '../../registry/block_codecs.dart';
import '../../registry/note_type_registry.dart';
import '../../ui/note_preview.dart';
import 'blocks/database.dart';
import 'blocks/view.dart';
import 'database_view.dart';
import 'properties/properties.dart';
import 'views/views.dart';

export 'blocks/database.dart' show databaseBlock, databaseKind;
export 'blocks/view.dart' show viewBlock, viewKind, PageChildrenView;
export 'controller.dart' show DbController;
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
    // A page with child pages opens in the editor like any page; its view
    // blocks show the children.
    ctx.registerNoteType(
      NoteType(
        id: 'database',
        label: 'Page',
        chip: noteChip,
        card: noteCard,
        embed: _embed,
        fullPage: (context, args) => args.editor,
      ),
    );
    ctx.blockKinds.register(viewBlock, before: 'callout');
    // Old inline database blocks keep rendering; the `/` menu offers View.
    ctx.blockKinds.register(
      databaseBlock.inGroup(databaseBlock.group, codecs: genericBlockCodecs),
      before: 'callout',
    );
    registerBuiltinProperties(ctx.properties);
    registerBuiltinViews(ctx.views);
  }
}

Widget _embed(BuildContext context, NoteArgs args) => LayoutBuilder(
  builder: (context, box) => DatabaseView(
    key: ValueKey('inline-db-${args.pageId}'),
    collection: args.pageId,
    inline: true,
    width: box.maxWidth,
  ),
);
