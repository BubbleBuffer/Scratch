import 'package:flutter/widgets.dart';

import '../../core/models.dart';
import '../../state/agenda_store.dart' show months;
import '../../ui/database/db_icons.dart';
import '../../ui/ui.dart';

/// The same small source-aware row in Today, Calendar, note cards and embeds.
class ShortItemCard extends StatelessWidget {
  const ShortItemCard({
    super.key,
    required this.item,
    this.calendars,
    this.sourceTitle,
    this.overdue = false,
    this.showDate = false,
    this.onOpen,
    this.onToggle,
    this.busy = false,
    this.checkbox = true,
    this.undated = false,
  });
  final AgendaItem item;
  final CalendarSettings? calendars;
  final String? sourceTitle;
  final bool overdue, showDate, busy, checkbox, undated;
  final VoidCallback? onOpen, onToggle;

  CalendarCategory? get category {
    for (final c in calendars?.calendars ?? <CalendarCategory>[]) {
      if (c.id == item.calendar) return c;
    }
    return null;
  }

  String get source => item.block == null && item.collection == null
      ? category?.name ?? 'Tasks'
      : sourceTitle ?? (item.collection != null ? 'Database' : 'Note');

  String get dateLabel {
    if (undated) return '';
    final d = DateTime.fromMillisecondsSinceEpoch(
      item.startMs.toInt(),
      isUtc: !item.hasTime,
    );
    final date = '${d.day} ${months[d.month - 1].substring(0, 3)}';
    if (overdue) return date;
    final time =
        '${d.hour.toString().padLeft(2, '0')}:${d.minute.toString().padLeft(2, '0')}';
    if (showDate) return item.hasTime ? '$date · $time' : date;
    return item.hasTime ? time : '';
  }

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final edge =
        spaceColor(
          category?.color ?? item.spaceColor,
          c.dark ? Brightness.dark : Brightness.light,
        ) ??
        c.threadStrong;
    final titleColor = item.done ? c.textTertiary : c.textPrimary;
    final title = Text(
      displayTitle(item.title),
      key: const ValueKey('short-item-title'),
      maxLines: 1,
      overflow: TextOverflow.ellipsis,
      style: ts(
        context.type.ui,
        titleColor,
        weight: 600,
      ).copyWith(decoration: item.done ? TextDecoration.lineThrough : null),
    );
    final time = dateLabel;
    final meta = Row(
      mainAxisSize: MainAxisSize.min,
      children: [
        if (item.hasTime && !overdue && !showDate) ...[
          Icon20(Ico.clock, size: 12, color: c.iconTertiary),
          const SizedBox(width: 4),
        ],
        Flexible(
          child: Text(
            time,
            maxLines: 1,
            overflow: TextOverflow.ellipsis,
            style: ts(
              context.type.uiSmall,
              overdue && !item.done ? c.inlineCodeText : c.textTertiary,
              mono: true,
            ),
          ),
        ),
      ],
    );
    final sourceWidget = _ItemSource(
      label: source,
      details:
          '${category?.name ?? 'Calendar'}${sourceTitle == null ? '' : ' · $sourceTitle'}',
      color: edge,
      task: item.block == null && item.collection == null,
      database: item.collection != null,
    );
    final agent = item.assignee == 'agent'
        ? Padding(
            padding: const EdgeInsets.only(left: 6),
            child: Text(
              'Agent',
              key: const ValueKey('short-item-agent'),
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
              style: ts(context.type.uiSmall, c.textTertiary),
            ),
          )
        : const SizedBox.shrink();
    final leading = item.kind == AgendaKind.task && checkbox
        ? Semantics(
            label: item.done
                ? 'Mark ${item.title} to do'
                : 'Complete ${item.title}',
            child: StacksCheckbox(
              checked: item.done,
              size: 18,
              onChanged: busy || onToggle == null ? null : (_) => onToggle!(),
            ),
          )
        : item.collection != null
        ? DbIcon(DbIco.database, size: 18, color: c.iconTertiary)
        : item.kind == AgendaKind.mention
        ? PageIcon(icon: item.icon, size: 18, color: c.iconTertiary)
        : Icon20(
            item.hasTime ? Ico.clock : Ico.todo,
            size: 18,
            color: c.iconTertiary,
          );
    return Semantics(
      label:
          '${item.title}${item.done ? ', done' : ''}${overdue ? ', overdue' : ''}',
      child: ClipRRect(
        borderRadius: Radii.ctl,
        child: Stack(
          children: [
            Hover(
              onTap: busy ? null : onOpen,
              background: c.bgPrimary,
              border: Border.all(color: c.borderSubtle),
              lift: 0,
              padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 6),
              child: LayoutBuilder(
                builder: (context, box) {
                  final compact = context.mobile;
                  final narrowMeta = box.maxWidth < 400
                      ? Text(
                          time,
                          maxLines: 1,
                          overflow: TextOverflow.ellipsis,
                          style: ts(
                            context.type.uiSmall,
                            overdue && !item.done
                                ? c.inlineCodeText
                                : c.textTertiary,
                            mono: true,
                          ),
                        )
                      : meta;
                  return Row(
                    crossAxisAlignment: CrossAxisAlignment.center,
                    children: [
                      Hover(
                        lift: 0,
                        onTap: item.kind == AgendaKind.task && checkbox && !busy
                            ? onToggle
                            : null,
                        child: SizedBox(
                          width: compact ? 44 : 26,
                          height: compact ? 44 : 24,
                          child: Center(child: leading),
                        ),
                      ),
                      const SizedBox(width: 4),
                      if (compact)
                        Expanded(
                          child: Column(
                            mainAxisSize: MainAxisSize.min,
                            crossAxisAlignment: CrossAxisAlignment.start,
                            children: [
                              title,
                              const SizedBox(height: 2),
                              Row(
                                children: [
                                  if (time.isNotEmpty) ...[
                                    meta,
                                    const SizedBox(width: 10),
                                  ],
                                  Flexible(child: sourceWidget),
                                  agent,
                                ],
                              ),
                            ],
                          ),
                        )
                      else if (box.maxWidth >= 520) ...[
                        Expanded(child: title),
                        agent,
                        if (time.isNotEmpty) ...[
                          const SizedBox(width: 12),
                          meta,
                        ],
                        const SizedBox(width: 16),
                        ConstrainedBox(
                          constraints: const BoxConstraints(maxWidth: 160),
                          child: sourceWidget,
                        ),
                      ] else ...[
                        Expanded(flex: 5, child: title),
                        if (item.assignee == 'agent')
                          Flexible(flex: 3, fit: FlexFit.loose, child: agent),
                        if (time.isNotEmpty) ...[
                          const SizedBox(width: 12),
                          Flexible(
                            flex: 3,
                            fit: FlexFit.loose,
                            child: narrowMeta,
                          ),
                        ],
                        const SizedBox(width: 16),
                        Flexible(
                          flex: 4,
                          fit: FlexFit.loose,
                          child: ConstrainedBox(
                            constraints: BoxConstraints(
                              maxWidth: (box.maxWidth * .38).clamp(0.0, 160.0),
                            ),
                            child: sourceWidget,
                          ),
                        ),
                      ],
                    ],
                  );
                },
              ),
            ),
            Positioned(
              left: 0,
              top: 0,
              bottom: 0,
              child: IgnorePointer(
                child: ColoredBox(color: edge, child: const SizedBox(width: 3)),
              ),
            ),
          ],
        ),
      ),
    );
  }
}

class _ItemSource extends StatefulWidget {
  const _ItemSource({
    required this.label,
    required this.details,
    required this.color,
    required this.task,
    required this.database,
  });
  final String label, details;
  final Color color;
  final bool task, database;
  @override
  State<_ItemSource> createState() => _ItemSourceState();
}

class _ItemSourceState extends State<_ItemSource> {
  final _portal = OverlayPortalController();
  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    return OverlayPortal(
      controller: _portal,
      overlayChildBuilder: (context) {
        final rect = globalRectOf(this.context);
        final width = MediaQuery.sizeOf(context).width;
        return Positioned(
          left: rect.left.clamp(8.0, (width - 258).clamp(8.0, width)),
          top: rect.bottom + 6,
          child: IgnorePointer(
            child: MenuSurface(
              width: 250,
              padding: const EdgeInsets.all(10),
              child: Text(
                widget.details,
                style: ts(context.type.uiSmall, c.textSecondary),
              ),
            ),
          ),
        );
      },
      child: MouseRegion(
        onEnter: (_) {
          if (!context.mobile) _portal.show();
        },
        onExit: (_) => _portal.hide(),
        child: Semantics(
          label: widget.details,
          button: true,
          child: Hover(
            onTap: () {
              _portal.hide();
              showModalPanel<void>(
                context,
                (_) => Padding(
                  padding: const EdgeInsets.all(16),
                  child: Text(
                    widget.details,
                    style: ts(context.type.ui, c.textSecondary),
                  ),
                ),
                width: 320,
                shrink: true,
              );
            },
            lift: 0,
            child: Row(
              mainAxisSize: MainAxisSize.min,
              children: [
                if (widget.task)
                  Container(
                    width: 6,
                    height: 6,
                    decoration: BoxDecoration(
                      color: widget.color,
                      shape: BoxShape.circle,
                    ),
                  )
                else if (widget.database)
                  DbIcon(DbIco.database, size: 12, color: c.iconTertiary)
                else
                  Icon20(Ico.page, size: 12, color: c.iconTertiary),
                const SizedBox(width: 5),
                Flexible(
                  child: Text(
                    widget.label,
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: ts(context.type.uiSmall, c.textTertiary),
                  ),
                ),
              ],
            ),
          ),
        ),
      ),
    );
  }
}
