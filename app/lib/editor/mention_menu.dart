// The `@` menu: dates ("Today", "Tomorrow", a typed date) and pages. A
// page becomes its title with a PageLink mark; a date becomes "@October 1,
// 2026" with a Date mark holding the ISO date (shown grey).

import 'package:flutter/widgets.dart';

import '../shell/page_stack.dart';
import '../ui/ui.dart';
import 'block_pickers.dart';

class MentionItem {
  const MentionItem.page(this.id, this.label, this.icon)
    : date = null,
      isChat = false;
  const MentionItem.date(DateTime this.date, this.label)
    : id = '',
      icon = null,
      isChat = false;
  const MentionItem.chat(this.id, this.label)
    : icon = null,
      date = null,
      isChat = true;
  final bool isChat;
  final String id, label;
  final String? icon;
  final DateTime? date;
  bool get isDate => date != null;
  String get key => isDate
      ? 'date-${isoDate(date!)}'
      : isChat
      ? 'chat-$id'
      : 'page-$id';
}

const _months = [
  'January', 'February', 'March', 'April', 'May', 'June', //
  'July', 'August', 'September', 'October', 'November', 'December',
];

String isoDate(DateTime d) =>
    '${d.year.toString().padLeft(4, '0')}-${d.month.toString().padLeft(2, '0')}-${d.day.toString().padLeft(2, '0')}';

/// "October 1, 2026".
String longDate(DateTime d) => '${_months[d.month - 1]} ${d.day}, ${d.year}';

/// Date entries for [query]: today/tomorrow/yesterday by prefix, an ISO
/// date, or "`<month> <day>[, year]`".
List<MentionItem> dateItems(String query, DateTime now) {
  final q = query.trim().toLowerCase();
  final today = DateTime(now.year, now.month, now.day);
  final out = <MentionItem>[];
  for (final (word, delta) in const [
    ('today', 0),
    ('tomorrow', 1),
    ('yesterday', -1),
  ]) {
    if (q.isEmpty ? delta >= 0 : word.startsWith(q)) {
      out.add(
        MentionItem.date(
          today.add(Duration(days: delta)),
          '${word[0].toUpperCase()}${word.substring(1)}',
        ),
      );
    }
  }
  final iso = RegExp(r'^(\d{4})-(\d{1,2})-(\d{1,2})$').firstMatch(q);
  if (iso != null) {
    final d = DateTime(
      int.parse(iso[1]!),
      int.parse(iso[2]!),
      int.parse(iso[3]!),
    );
    out.add(MentionItem.date(d, longDate(d)));
  }
  final md = RegExp(r'^([a-z]+)\s+(\d{1,2})(?:,?\s*(\d{4}))?$').firstMatch(q);
  if (md != null) {
    final mi = _months.indexWhere((m) => m.toLowerCase().startsWith(md[1]!));
    final day = int.parse(md[2]!);
    if (mi >= 0 && day >= 1 && day <= 31) {
      final d = DateTime(
        md[3] == null ? now.year : int.parse(md[3]!),
        mi + 1,
        day,
      );
      out.add(MentionItem.date(d, longDate(d)));
    }
  }
  return out;
}

class MentionMenu extends StatelessWidget {
  const MentionMenu({
    super.key,
    required this.items,
    required this.active,
    required this.onPick,
    required this.onHover,
    this.maxHeight = 360,
  });
  final List<MentionItem> items;
  final int active;
  final ValueChanged<MentionItem> onPick;
  final ValueChanged<int> onHover;
  final double maxHeight;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final m = context.mobile;
    final rows = <Widget>[];
    String? section;
    for (var i = 0; i < items.length; i++) {
      final it = items[i];
      final s = it.isDate
          ? 'Date'
          : it.isChat
          ? 'Link to chat'
          : 'Link to page';
      if (s != section) {
        if (section != null) rows.add(const MenuSeparator());
        rows.add(MenuHeader(s));
        section = s;
      }
      final page = it.isDate || it.isChat
          ? null
          : pageLabelOf(context, it.id, it.label);
      rows.add(
        MenuItem(
          key: ValueKey('mention-${it.key}'),
          label: it.isChat
              ? it.label
              : page?.text ??
                    '${it.label}${it.label == longDate(it.date!) ? '' : '  ·  ${longDate(it.date!)}'}',
          firstLine: page?.firstLine ?? false,
          leading: it.isDate
              ? Icon20(Ico.clock, size: m ? 22 : 20, color: c.iconSecondary)
              : it.isChat
              ? Icon20(Ico.comment, size: m ? 22 : 20, color: c.iconSecondary)
              : PageIcon(icon: it.icon, size: m ? 22 : 20),
          selected: i == active,
          onTap: () => onPick(it),
          onHover: () => onHover(i),
        ),
      );
    }
    if (items.isEmpty) rows.add(const MenuHeader('No results'));
    return MenuSurface(
      width: m ? null : 324,
      padding: const EdgeInsets.all(6),
      child: ConstrainedBox(
        constraints: BoxConstraints(maxHeight: maxHeight - 12),
        child: SingleChildScrollView(
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: rows,
          ),
        ),
      ),
    );
  }
}

/// Pages for the `@` menu.
Future<List<MentionItem>> mentionPages(
  BuildContext context,
  String query,
  String exclude,
) async {
  final hits = await searchPages(context, query, limit: 6);
  return [
    for (final h in hits)
      if (h.id != exclude) MentionItem.page(h.id, h.title, h.icon),
  ];
}
