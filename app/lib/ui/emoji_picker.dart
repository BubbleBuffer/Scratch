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
    this.width = 408,
    this.height = 356,
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
    const cell = 32.0;
    final emojiStyle = TextStyle(
      fontSize: 22,
      height: 1,
      fontFamilyFallback: emojiFallback,
    );

    return Column(
      mainAxisSize: MainAxisSize.min,
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        // Tabs.
        Container(
          height: 40,
          padding: const EdgeInsets.symmetric(horizontal: 8),
          decoration: BoxDecoration(
            border: Border(bottom: BorderSide(color: c.border)),
          ),
          child: Row(
            children: [
              Container(
                height: 40,
                padding: const EdgeInsets.symmetric(horizontal: 6),
                alignment: Alignment.center,
                decoration: BoxDecoration(
                  border: Border(
                    bottom: BorderSide(color: c.textPrimary, width: 2),
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
          padding: const EdgeInsets.fromLTRB(12, 10, 12, 4),
          child: Row(
            children: [
              Expanded(
                child: FieldFrame(
                  height: 28,
                  child: PlainField(
                    key: const ValueKey('emoji-filter'),
                    controller: _query,
                    focusNode: _focus,
                    autofocus: !context.mobile,
                    style: ts(ty.ui, c.textPrimary, weight: 400),
                    placeholder: 'Filter…',
                    onChanged: (_) => setState(() {}),
                  ),
                ),
              ),
              const SizedBox(width: 8),
              Hover(
                key: const ValueKey('emoji-random'),
                onTap: _random,
                radius: Radii.md,
                child: Container(
                  width: 28,
                  height: 28,
                  alignment: Alignment.center,
                  decoration: BoxDecoration(
                    border: Border.all(color: c.border),
                    borderRadius: Radii.ctl,
                  ),
                  child: Text('🎲', style: emojiStyle.copyWith(fontSize: 16)),
                ),
              ),
            ],
          ),
        ),
        SizedBox(
          height: widget.height - 40 - 42,
          child: CustomScrollView(
            slivers: [
              for (final (name, list) in sections) ...[
                SliverToBoxAdapter(
                  child: Padding(
                    padding: const EdgeInsets.fromLTRB(14, 10, 14, 4),
                    child: Text(name, style: ts(ty.uiSmall, c.textSecondary)),
                  ),
                ),
                SliverPadding(
                  padding: const EdgeInsets.symmetric(horizontal: 10),
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
                        child: Center(child: Text(list[i], style: emojiStyle)),
                      ),
                      childCount: list.length,
                    ),
                  ),
                ),
              ],
              const SliverToBoxAdapter(child: SizedBox(height: 8)),
            ],
          ),
        ),
      ],
    );
  }
}

/// Shows the emoji picker by [anchor] (desktop) or as a sheet (mobile).
/// Completes with the emoji, '' for "Remove", or null when dismissed.
Future<String?> showEmojiPicker(
  BuildContext context, {
  Rect? anchor,
  bool removable = true,
}) {
  if (context.mobile || anchor == null) {
    return showSheet<String>(
      context,
      (ctx) => SheetSurface(
        children: [
          EmojiPicker(
            key: const ValueKey('emoji-picker'),
            width: MediaQuery.of(ctx).size.width,
            height: MediaQuery.of(ctx).size.height * 0.6,
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
      width: 408,
      child: EmojiPicker(
        key: const ValueKey('emoji-picker'),
        onPick: (e) => Navigator.of(ctx).pop(e),
        onRemove: removable ? () => Navigator.of(ctx).pop('') : null,
      ),
    ),
  );
}
