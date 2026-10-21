import 'package:flutter/widgets.dart';

import '../core/vault.dart';
import '../ui/embed.dart';
import '../ui/icon_x.dart';
import '../ui/ui.dart';
import 'addon_registry.dart';
import 'block_codecs.dart';
import 'block_registry.dart';

const embedKind = 'embed';

/// A host may resolve current metadata while exporting synchronously. The
/// result is visible Markdown only; an embed never writes it to block text.
String? Function(String id)? embedTitleResolver;

final _titleResolvers = <String? Function(String)>[];
void addEmbedTitleResolver(String? Function(String) resolver) =>
    _titleResolvers.add(resolver);
void removeEmbedTitleResolver(String? Function(String) resolver) =>
    _titleResolvers.remove(resolver);

// Presentation metadata only, keyed weakly by its open vault. Nothing is
// retained across vault lifetimes or written into the embed's block text.
final _embedTitles = Expando<Map<String, String>>('embed titles');
String? embedTitleOf(Vault vault, String id) => _embedTitles[vault]?[id];
void rememberEmbedTitle(Vault vault, String id, String title) =>
    (_embedTitles[vault] ??= {})[id] = title;

String _fallback(String? target) => target == null || target.isEmpty
    ? ''
    : (embedTitleResolver ?? _titleResolvers.lastOrNull)?.call(target) ?? '';

final embedBlockCodecs = BlockCodecs(
  markdown: MarkdownCodec(
    (n, _) => annotatedBlockMarkdown(n, _fallback(n.target)),
  ),
  clipboard: const ClipboardCodec.viaMarkdown(),
  plainText: PlainTextCodec((n) => _fallback(n.target)),
);

/// Shared kit registration. It is available even with no add-ons installed.
void registerEmbedBlock(AddonContext ctx) {
  ctx.blockKinds.register(
    BlockType(
      kind: embedKind,
      label: 'Embed',
      hint: 'Show another note here.',
      iconX: Ix.linkPage,
      aliases: 'embed note page document database chat',
      group: 'Advanced',
      editable: false,
      acceptsChildren: false,
      colorable: false,
      handleTop: (b) => 6,
      codecs: embedBlockCodecs,
      body: (b) {
        final target = b.node.target;
        if (target == null || target.isEmpty) {
          return Hover(
            key: ValueKey('empty-embed-${b.node.id}'),
            onTap: () => b.host.openBlockPicker(b.node, b.context),
            child: Padding(
              padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 10),
              child: Text(
                'Choose a note to embed',
                style: ts(b.type.ui, b.colors.textTertiary),
              ),
            ),
          );
        }
        return EmbedNote(
          key: ValueKey('embed-${b.node.id}'),
          targetId: target,
          rootPageId: b.host.pageId,
          instanceId: b.node.id,
          addons: ctx,
          onOpenPage: b.host.openPage,
        );
      },
    ),
  );
}
