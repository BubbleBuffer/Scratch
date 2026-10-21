// Today agenda: one continuous, scrollable list of days around today, with a week strip
// that follows the topmost visible day. Publishes window.createTodayAgenda(ctx).
// Cards are rendered only through ctx.itemCard; buttons only through ctx.button.
(function () {
  const RANGE_DAYS = 21;
  const prefersReducedMotion = () =>
    Boolean(window.matchMedia && window.matchMedia('(prefers-reduced-motion: reduce)').matches);

  function createTodayAgenda(ctx) {
    const today = ctx.today;
    const tomorrow = ctx.add(today, 1);
    // The rendered range only grows, so a rerender never shrinks the scroller under the user.
    const range = { start: ctx.add(today, -RANGE_DAYS), end: ctx.add(today, RANGE_DAYS) };

    let selected = today;
    let scroller = null;
    let strip = null;
    let monthLabel = null;
    let backButton = null;
    let overdueEl = null;
    // Kept across rerenders; updated from the details element's toggle event.
    let overdueOpen = true;
    let frame = 0;

    const monthYear = (date) =>
      new Date(date + 'T00:00:00Z').toLocaleDateString('en-US', { month: 'long', year: 'numeric', timeZone: 'UTC' });

    // A date outside the range extends it to date ± 21 days; returns true when it changed.
    function ensureRange(date) {
      if (date >= range.start && date <= range.end) return false;
      const from = ctx.add(date, -RANGE_DAYS);
      const to = ctx.add(date, RANGE_DAYS);
      if (from < range.start) range.start = from;
      if (to > range.end) range.end = to;
      return true;
    }

    function dayList() {
      const days = [];
      for (let d = range.start; d <= range.end; d = ctx.add(d, 1)) days.push(d);
      return days;
    }

    const cardRow = (item, overdue = false) => `<div class="today-item" role="listitem">${ctx.itemCard(item, { overdue })}</div>`;

    function overdueHtml() {
      const items = ctx.overdue();
      const rows = items.length
        ? items.map(item => cardRow(item, true)).join('')
        : '<p class="today-empty">Nothing overdue.</p>';
      return `<details class="today-overdue"${overdueOpen ? ' open' : ''}>
        <summary class="today-overdue-summary">Overdue · ${items.length}</summary>
        <div class="today-day-body" role="list">${rows}</div>
      </details>`;
    }

    function dayHtml(date) {
      const items = ctx.itemList(date);
      const tag = date === today ? 'Today' : date === tomorrow ? 'Tomorrow' : '';
      const label = ctx.shortDate(date); // already includes the weekday
      const rows = items.map(item => cardRow(item)).join('');
      const body = (date === today ? overdueHtml() : '') + (rows || '<p class="today-empty">Nothing planned</p>');
      const title = `<span class="today-day-date">${ctx.esc(label)}</span>`
        + (tag ? `<span class="today-day-sep" aria-hidden="true">·</span><span class="today-day-tag">${tag}</span>` : '');
      return `<section class="today-day${date === today ? ' is-today' : date === ctx.add(today, -1) ? ' is-past is-yesterday' : date < today ? ' is-past' : ''}" data-date="${date}" aria-labelledby="today-day-${date}">
        <div class="today-day-header">
          <h3 class="today-day-title" id="today-day-${date}">${title}</h3>
          ${ctx.button('create', 'Add task', 'plus', 'today-add', `data-date="${date}" aria-label="Add task for ${ctx.esc(label)}"`)}
        </div>
        <div class="today-day-body" role="list">${body}</div>
      </section>`;
    }

    const scrollerBody = () => dayList().map(dayHtml).join('');

    function toolbarHtml(date) {
      return `<header class="calendar-header today-header document-top">
        <h1 class="header-title">Today</h1>
        <div class="today-toolbar-nav">
          ${ctx.button('jump', monthYear(ctx.monday(date)), '', 'today-month', 'aria-label="Jump to date" title="Jump to date (G)" aria-live="polite"')}
          ${ctx.button('today-week-prev', '', 'left', 'today-nav', 'aria-label="Previous week" title="Previous week"')}
          ${ctx.button('today-week-next', '', 'right', 'today-nav', 'aria-label="Next week" title="Next week"')}
        </div>
        ${ctx.button('back-to-today', 'Today', '', 'today-back', 'hidden')}
        <div class="header-actions">
          ${ctx.button('jump', '', 'calendar', 'icon-btn', 'aria-label="Jump to date" title="Jump to date (G)"')}
          ${ctx.button('calendars', '', 'settings', 'icon-btn', 'aria-label="Calendars" title="Calendars"')}
          ${ctx.button('pages', '', 'more', 'icon-btn', 'aria-label="More screens"')}
        </div>
      </header>`;
    }

    function html(date) {
      ensureRange(date);
      return `<div class="today-view">
        <div id="today-week-strip" class="today-week-strip">${ctx.weekStrip(date)}</div>
        <div id="today-scroll" class="today-scroll" role="region" aria-label="Agenda, 21 days either side of today" tabindex="0">${scrollerBody()}</div>
      </div>`;
    }

    // Keeps the week strip, month label and selection in step with the topmost day.
    function select(date) {
      if (date === selected) return;
      selected = date;
      ctx.onDateChange(date);
      if (strip) strip.innerHTML = ctx.weekStrip(date);
      if (monthLabel) monthLabel.textContent = monthYear(ctx.monday(date));
    }

    // Every scroll position, including intermediate days during a smooth scroll, selects the
    // topmost day whose section has reached the top of the scroller.
    function sync() {
      frame = 0;
      if (!scroller) return;
      const pos = scroller.scrollTop;
      const days = scroller.querySelectorAll('.today-day');
      let topmost = days[0] || null;
      for (const el of days) {
        if (el.offsetTop <= pos + 1) topmost = el;
        else break;
      }
      const todayEl = scroller.querySelector(`.today-day[data-date="${today}"]`);
      const peeking = todayEl && todayEl.offsetTop - pos > 0 && todayEl.offsetTop - pos <= 65;
      scroller.classList.toggle('peeking-yesterday', Boolean(peeking));
      if (peeking) topmost = todayEl;
      if (backButton) {
        const outside = !todayEl
          || todayEl.offsetTop + todayEl.offsetHeight <= pos
          || todayEl.offsetTop >= pos + scroller.clientHeight;
        backButton.hidden = !outside;
      }
      if (topmost) select(topmost.dataset.date);
    }

    function onScroll() {
      if (!frame) frame = requestAnimationFrame(sync);
    }

    function onOverdueToggle(event) {
      overdueOpen = event.currentTarget.open;
    }

    // Re-finds the overdue details after the day list is rebuilt and keeps its state in sync.
    function bindOverdue() {
      if (overdueEl) overdueEl.removeEventListener('toggle', onOverdueToggle);
      overdueEl = scroller ? scroller.querySelector('.today-overdue') : null;
      if (overdueEl) overdueEl.addEventListener('toggle', onOverdueToggle);
    }

    function detach() {
      if (scroller) scroller.removeEventListener('scroll', onScroll);
      if (overdueEl) overdueEl.removeEventListener('toggle', onOverdueToggle);
      overdueEl = null;
      if (frame) cancelAnimationFrame(frame);
      frame = 0;
    }

    // Let the final day reach the sticky-header position even when it has few items.
    function padLastDay() {
      if (!scroller) return;
      const last = scroller.querySelector('.today-day:last-child');
      if (last) scroller.style.paddingBottom = Math.max(48, scroller.clientHeight - last.offsetHeight) + 'px';
    }

    function mount(root, date = today, savedScrollTop = null) {
      detach();
      if (!root.querySelector('.today-view')) root.innerHTML = html(date);
      ensureRange(date);
      scroller = root.querySelector('#today-scroll');
      strip = root.querySelector('#today-week-strip');
      monthLabel = root.querySelector('.today-month');
      backButton = root.querySelector('[data-action="back-to-today"]');
      if (!scroller) return;
      if (!scroller.querySelector(`.today-day[data-date="${date}"]`)) scroller.innerHTML = scrollerBody();
      bindOverdue();
      padLastDay();
      selected = date;
      if (strip) strip.innerHTML = ctx.weekStrip(date);
      if (monthLabel) monthLabel.textContent = monthYear(ctx.monday(date));
      if (savedScrollTop !== null) scroller.scrollTop = savedScrollTop;
      else scroller.scrollTop = Math.max(0, scroller.querySelector(`.today-day[data-date="${date}"]`).offsetTop - (date === today ? 64 : 0));
      scroller.addEventListener('scroll', onScroll, { passive: true });
      sync();
    }

    function scrollTo(date, smooth = true) {
      if (!scroller || !date) return;
      if (!scroller.querySelector(`.today-day[data-date="${date}"]`)) {
        // Outside the rendered range: extend it, rebuild the day list once and show the new range.
        ensureRange(date);
        scroller.innerHTML = scrollerBody();
        bindOverdue();
      }
      padLastDay();
      const target = scroller.querySelector(`.today-day[data-date="${date}"]`);
      if (!target) return;
      const animate = smooth && !prefersReducedMotion();
      scroller.scrollTo({ top: target.offsetTop, behavior: animate ? 'smooth' : 'auto' });
      // An instant jump is final already, so update the selection now; smooth scrolls update through scroll events.
      if (!animate) sync();
    }

    function destroy() {
      detach();
      scroller = strip = monthLabel = backButton = null;
    }

    return { html, toolbarHtml, mount, scrollTo, destroy };
  }

  window.createTodayAgenda = createTodayAgenda;
})();
