// The emoji picker (page icons, callout icons): an "Emoji" tab with
// "Remove" on the right, a filter field with a random button, then the
// emoji in a grid under category headers. A popover on desktop (408 wide), a sheet on mobile.

import 'dart:math' as math;

import 'package:flutter/widgets.dart';

import 'block_colors.dart';
import 'emoji_data.dart';
import 'ui.dart';

class EmojiPicker extends StatefulWidget {
  const EmojiPicker({
    super.key,
    required this.onPick,
    this.onRemove,
    this.width = EmojiPickerSize.width,
    this.height = EmojiPickerSize.height,
  });
  final ValueChanged<String> onPick;
  final VoidCallback? onRemove;
  final double width, height;

  @override
  State<EmojiPicker> createState() => _EmojiPickerState();
}

class _EmojiPickerState extends State<EmojiPicker> {
  final _query = TextEditingController();
  final _focus = FocusNode();

  @override
  void dispose() {
    _query.dispose();
    _focus.dispose();
    super.dispose();
  }

  void _random() {
    final r = math.Random();
    widget.onPick(allEmoji[r.nextInt(allEmoji.length)].$1);
  }

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final ty = context.type;
    final q = _query.text.trim().toLowerCase();
    final sections = <(String, List<String>)>[];
    if (q.isEmpty) {
      for (final cat in emojiCategories) {
        sections.add((
          cat.name,
          [for (final e in cat.entries) e.substring(0, e.indexOf(' '))],
        ));
      }
    } else {
      final hits = [
        for (final (e, words) in allEmoji)
          if (words.split(' ').any((w) => w.startsWith(q)) || words.contains(q))
            e,
      ];
      sections.add((hits.isEmpty ? 'No results' : 'Results', hits));
    }
    const cell = EmojiPickerSize.cell;
    final glyphStyle = emojiStyle(EmojiPickerSize.glyph, emojiFallback);

    return Column(
      mainAxisSize: MainAxisSize.min,
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        // Tabs.
        Container(
          height: EmojiPickerSize.tabBar,
          padding: const EdgeInsets.symmetric(horizontal: Space.sm),
          decoration: BoxDecoration(
            border: Border(bottom: BorderSide(color: c.border)),
          ),
          child: Row(
            children: [
              Container(
                height: EmojiPickerSize.tabBar,
                padding: const EdgeInsets.symmetric(horizontal: Space.xs),
                alignment: Alignment.center,
                decoration: BoxDecoration(
                  border: Border(
                    bottom: BorderSide(color: c.textPrimary, width: Stroke.bold),
                  ),
                ),
                child: Text('Emoji', style: ts(ty.ui, c.textPrimary)),
              ),
              const Spacer(),
              if (widget.onRemove != null)
                TextBtn(
                  'Remove',
                  key: const ValueKey('emoji-remove'),
                  onTap: widget.onRemove,
                  color: c.textSecondary,
                ),
            ],
          ),
        ),
        // Filter and random.
        Padding(
          padding: const EdgeInsets.fromLTRB(Space.md, Space.md, Space.md, Space.xxs),
          child: Row(
            children: [
              Expanded(
                child: FieldFrame(
                  height: ControlSize.sm.height,
                  child: PlainField(
                    key: const ValueKey('emoji-filter'),
                    controller: _query,
                    focusNode: _focus,
                    autofocus: !context.touch,
                    style: ts(ty.ui, c.textPrimary, weight: Weight.regular),
                    placeholder: 'Filter…',
                    onChanged: (_) => setState(() {}),
                  ),
                ),
              ),
              const Gap(Space.sm),
              Hover(
                key: const ValueKey('emoji-random'),
                onTap: _random,
                radius: Radii.md,
                child: Container(
                  width: ControlSize.sm.height,
                  height: ControlSize.sm.height,
                  alignment: Alignment.center,
                  decoration: BoxDecoration(
                    border: Border.all(color: c.border),
                    borderRadius: Radii.ctl,
                  ),
                  child: Text('🎲', style: emojiStyle(EmojiPickerSize.dice, emojiFallback)),
                ),
              ),
            ],
          ),
        ),
        SizedBox(
          height: widget.height - EmojiPickerSize.tabBar - EmojiPickerSize.filterRow,
          child: CustomScrollView(
            slivers: [
              for (final (name, list) in sections) ...[
                SliverToBoxAdapter(
                  child: Padding(
                    padding: const EdgeInsets.fromLTRB(Space.lg, Space.md, Space.lg, Space.xxs),
                    child: Text(name, style: ts(ty.uiSmall, c.textSecondary)),
                  ),
                ),
                SliverPadding(
                  padding: const EdgeInsets.symmetric(horizontal: Space.md),
                  sliver: SliverGrid(
                    gridDelegate:
                        const SliverGridDelegateWithMaxCrossAxisExtent(
                          maxCrossAxisExtent: cell,
                          mainAxisExtent: cell,
                        ),
                    delegate: SliverChildBuilderDelegate(
                      (context, i) => Hover(
                        key: ValueKey('emoji-${list[i]}'),
                        radius: Radii.sm,
                        onTap: () => widget.onPick(list[i]),
                        child: Center(child: Text(list[i], style: glyphStyle)),
                      ),
                      childCount: list.length,
                    ),
                  ),
                ),
              ],
              const SliverToBoxAdapter(child: SizedBox(height: Space.sm)),
            ],
          ),
        ),
      ],
    );
  }
}

/// Shows the emoji picker by [anchor], or as a sheet on a compact screen.
/// Completes with the emoji, '' for "Remove", or null when dismissed.
Future<String?> showEmojiPicker(
  BuildContext context, {
  Rect? anchor,
  bool removable = true,
}) {
  if (context.sizeClass.isCompact || anchor == null) {
    return showSheet<String>(
      context,
      (ctx) => SheetSurface(
        children: [
          EmojiPicker(
            key: const ValueKey('emoji-picker'),
            width: MediaQuery.of(ctx).size.width,
            height: MediaQuery.of(ctx).size.height * EmojiPickerSize.sheetShare,
            onPick: (e) => Navigator.of(ctx).pop(e),
            onRemove: removable ? () => Navigator.of(ctx).pop('') : null,
          ),
        ],
      ),
    );
  }
  return showPopover<String>(
    context,
    anchor,
    (ctx) => MenuSurface(
      width: EmojiPickerSize.width,
      child: EmojiPicker(
        key: const ValueKey('emoji-picker'),
        onPick: (e) => Navigator.of(ctx).pop(e),
        onRemove: removable ? () => Navigator.of(ctx).pop('') : null,
      ),
    ),
  );
}
