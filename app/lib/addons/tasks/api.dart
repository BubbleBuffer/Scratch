import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';

import '../../registry/addon_registry.dart';
import '../../registry/command_registry.dart';
import '../../registry/note_type_registry.dart';
import '../../services/addon_services.dart';
import '../../state/app_state.dart';
import '../../ui/ui.dart';
import 'calendar_screen.dart';
import 'navigation.dart';
import 'task_data.dart';
import 'task_note.dart';
import 'title_dates.dart';

export 'calendar_screen.dart'
    show
        VaultCalendarScreen,
        CalendarJumpPicker,
        openCalendar,
        calendarRouteName;
export 'categories.dart' show showCalendarCategories;
export 'item_card.dart' show ShortItemCard;
export 'navigation.dart' show openTask, showTaskComposer;
export 'task_composer.dart' show TaskComposer;
export 'task_data.dart'
    show
        TaskData,
        loadTask,
        saveTask,
        rescheduleTask,
        taskDateString,
        taskType,
        taskAddonId;
export 'task_note.dart' show TaskNote, TaskPresentation;

class TasksAddon extends Addon {
  const TasksAddon();
  @override
  String get id => 'tasks';
  @override
  List<String> get dependsOn => const [];
  @override
  void register(AddonContext ctx) {
    ctx.titleChips.register(
      AddonTitleChip(id: 'tasks.dates', build: taskTitleDates),
    );
    ctx.registerNoteType(
      NoteType(
        id: taskType,
        label: 'Task',
        chip: taskChip,
        card: taskCard,
        boardCard: taskCard,
        embed: taskEmbed,
        fullPage: taskPage,
        commands: [
          Command(
            id: 'task.new',
            label: 'New task',
            icon: Ico.plus,
            run: (context) {
              _showNewTask(context);
            },
          ),
        ],
      ),
    );
    ctx.commands.register(
      Command(
        id: 'calendar.open',
        label: 'Calendar',
        icon: Ico.clock,
        shortcuts: const [
          SingleActivator(LogicalKeyboardKey.keyC, control: true, shift: true),
        ],
        run: (context) {
          openCalendar(context);
        },
      ),
    );
    ctx.shellNavigation.register(
      AddonShellNavigation(
        id: 'calendar',
        label: 'Calendar',
        icon: Ico.clock,
        routeName: calendarRouteName,
        mobileDock: true,
        open: (context) {
          openCalendar(context);
        },
      ),
    );
    ctx.shellPrimaryActions.register(
      AddonShellPrimaryAction(
        id: 'task.new',
        label: 'New task',
        destinations: const {'today', 'calendar'},
        run: (context) {
          _showNewTask(context);
        },
      ),
    );
  }
}

void _showNewTask(BuildContext context) {
  final route =
      PageStackScope.maybeOf(context)?.currentRoute ?? ModalRoute.of(context);
  showTaskComposer(
    context,
    date: context.app.agenda.selectedDay,
    fromToday: route?.isFirst ?? false,
  );
}
