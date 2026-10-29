// Progressive chart controls. All values and fallback descriptions come from Rust.
(() => {
  'use strict';
  // Owner dashboard preferences only. The public tracker never uses storage.
  const preferenceValues = {
    period: ['day','yesterday','24h','7d','28d','30d','91d','month','lastmonth','year','6mo','12mo','all'],
    compare: ['off','previous','year'],
    compare_match: ['date','weekday']
  };
  const reportPreferences = {
    metric: ['visitors','visits','pageviews','views_per_visit','bounce_rate','visit_duration','time_on_page','scroll_depth'],
    pages_tab: ['page','entry','exit'],
    sources_tab: ['channel','source','utm_medium','utm_source','utm_campaign','utm_content','utm_term'],
    locations_tab: ['map','country','region','city'],
    devices_tab: ['browser','os','device']
  };
  const intervalPeriod = period => ({yesterday:'day',lastmonth:'month'})[period] || period;
  const intervalPeriods = [...new Set(preferenceValues.period.map(intervalPeriod)),'custom'];
  const intervalValues = ['minute','hour','day','week','month'];
  const dashboard = () => document.querySelector('.dashboard-toolbar');
  const preferenceKey = () => {
    const site = document.querySelector('.dashboard-toolbar')?.dataset.site;
    return site ? `owlytics:dashboard:${site}` : null;
  };
  function preferences() {
    try {
      const raw = localStorage.getItem(preferenceKey());
      const stored = raw && raw.length < 2048 ? JSON.parse(raw) : {};
      const result = Object.fromEntries(Object.entries({...preferenceValues,...reportPreferences}).filter(([key, values])=>values.includes(stored?.[key])).map(([key])=>[key,stored[key]]));
      const intervals = Object.fromEntries(intervalPeriods.filter(period=>intervalValues.includes(stored?.intervals?.[period])).map(period=>[period,stored.intervals[period]]));
      if (Object.keys(intervals).length) result.intervals = intervals;
      return result;
    } catch { return {}; }
  }
  function writePreferences(stored) {
    try { localStorage.setItem(preferenceKey(), JSON.stringify(stored)); } catch { /* Storage is optional. */ }
  }
  function rememberChoice(link) {
    const name = link.dataset.preference;
    if (!name || !preferenceKey()) return;
    const stored = preferences(), params = new URL(link.getAttribute('href'),location.href).searchParams;
    let value = params.get(name);
    if (name === 'interval') {
      const period = intervalPeriod(dashboard()?.dataset.period);
      if (!intervalPeriods.includes(period) || !intervalValues.includes(value)) return;
      stored.intervals = {...stored.intervals,[period]:value};
    } else {
      value = ({referrer:'source',browser_version:'browser',os_version:'os'})[value] || value;
      if (!reportPreferences[name]?.includes(value)) return;
      stored[name] = value;
    }
    writePreferences(stored);
  }
  function savePreferences() {
    const key = preferenceKey();
    if (!key) return;
    const params = new URL(location.href).searchParams, stored = preferences();
    let changed = false;
    for (const [name, values] of Object.entries(preferenceValues)) {
      const value = params.get(name);
      if (values.includes(value) && stored[name] !== value) { stored[name] = value; changed = true; }
    }
    if (changed) writePreferences(stored);
  }
  const restoredURL = new URL(location.href);
  if (preferenceKey()) {
    const stored = preferences(), params = restoredURL.searchParams;
    for (const name of Object.keys(preferenceValues)) {
      if (!stored[name] || params.has(name)) continue;
      if (name === 'period' && ['date','from','to'].some(key=>params.has(key))) continue;
      params.set(name,stored[name]);
    }
    // Availability belongs to the destination dates/comparison, not the first
    // server render. This also keeps imported history from gaining fine buckets.
    if (restoredURL.href !== location.href) { location.replace(restoredURL); return; }
    let filters = [];
    try { const value = JSON.parse(params.get('filters') || '[]'); if (Array.isArray(value)) filters = value; } catch { /* The server validates filters. */ }
    const filtered = keys => keys.some(key=>params.has(key) || filters.some(filter=>filter.key === key));
    for (const [name,saved] of Object.entries(stored)) {
      if (!reportPreferences[name]) continue;
      if (restoredURL.searchParams.has(name)) continue;
      // A linked report or parent filter owns its drilldown, even when another
      // tab was preferred on the last unfiltered visit.
      if (name === 'locations_tab' && (filtered(['country','region','city']) || ['country','region','city'].includes(params.get('detail')))) continue;
      if (name === 'sources_tab' && (filtered(['source','channel']) || ['referrer',...reportPreferences.sources_tab].includes(params.get('detail')))) continue;
      if (name === 'devices_tab' && (filtered(['browser','os']) || ['browser_version','os_version',...reportPreferences.devices_tab].includes(params.get('detail')))) continue;
      if (name === 'pages_tab' && reportPreferences.pages_tab.includes(params.get('detail'))) continue;
      let value = saved;
      if (name === 'metric') {
        const available = [...document.querySelectorAll('a.metric[data-available=true]')].map(link=>new URL(link.href).searchParams.get('metric'));
        if (!available.includes(value)) { value = 'visitors'; stored.metric = value; writePreferences(stored); }
      }
      params.set(name,value);
    }
    const period = intervalPeriod(params.get('period') || dashboard().dataset.period);
    const interval = stored.intervals?.[period];
    // A restored date preset may need its own render before interval validity
    // can be checked (custom ranges, comparisons and imported daily history).
    if (!params.has('interval') && period === intervalPeriod(dashboard().dataset.period) && interval &&
        [...document.querySelectorAll('[data-preference=interval]')].some(link=>new URL(link.href).searchParams.get('interval') === interval)) params.set('interval',interval);
    if (restoredURL.href !== location.href) { location.replace(restoredURL); return; }
  }
  let controls;
  function initializeRowComparisons(signal) {
    const tooltip=document.querySelector('#row-comparison');
    if (!tooltip) return;
    const close=tooltip.querySelector('button');
    let selected=null,described=null,pinned=false,leaveTimer,suppressFocus=false;
    let hoverDismissed=false,pointerX,pointerY;
    for (const value of document.querySelectorAll('[data-row-comparison]')) {
      value.removeAttribute('title');
      value.closest('.data-row,td')?.removeAttribute('title');
      if (value.hasAttribute('tabindex')) value.setAttribute('role','button');
    }
    const hide=(restore=false)=>{
      const focus=selected?.closest('a')||selected;
      described?.removeAttribute('aria-describedby');
      selected?.removeAttribute('aria-expanded');
      clearTimeout(leaveTimer);tooltip.hidden=true;selected=null;described=null;pinned=false;
      if (restore) hoverDismissed=true;
      if (restore && focus?.isConnected) {suppressFocus=true;focus.focus({preventScroll:true});suppressFocus=false;}
    };
    const place=()=>{
      if (!selected || tooltip.hidden) return;
      const anchor=selected.getBoundingClientRect(),box=tooltip.getBoundingClientRect();
      if (anchor.bottom<0 || anchor.top>innerHeight) {hide();return;}
      tooltip.style.left=`${Math.max(8,Math.min(innerWidth-box.width-8,anchor.right-box.width))}px`;
      const below=anchor.bottom+8;
      tooltip.style.top=`${Math.max(8,Math.min(innerHeight-box.height-8,below+box.height<=innerHeight-8?below:anchor.top-box.height-8))}px`;
    };
    const show=(value,pin=false)=>{
      if (pinned && !pin || suppressFocus) return;
      if (selected!==value) described?.removeAttribute('aria-describedby');
      clearTimeout(leaveTimer);selected=value;pinned=pin;
      const data=value.dataset;
      tooltip.querySelector('.row-tooltip-heading strong').textContent=data.name;
      tooltip.querySelector('.row-tooltip-metric').textContent=data.heading;
      tooltip.querySelector('.row-current-date').textContent=data.currentDate;
      tooltip.querySelector('.row-current-value').textContent=data.current;
      tooltip.querySelector('.row-previous-date').textContent=data.previousDate;
      tooltip.querySelector('.row-previous-value').textContent=data.previous;
      const change=tooltip.querySelector('.row-tooltip-change');
      change.textContent=data.change;change.className=`row-tooltip-change ${data.tone}`;change.hidden=!data.change;
      tooltip.setAttribute('role',pin?'dialog':'tooltip');
      if (pin) tooltip.setAttribute('aria-label','Row comparison');else tooltip.removeAttribute('aria-label');
      close.hidden=!pin;tooltip.hidden=false;
      described=value.closest('a')||value;described.setAttribute('aria-describedby','row-comparison');
      if (value.getAttribute('role')==='button') value.setAttribute('aria-expanded',String(pin));
      place();if (pin) close.focus({preventScroll:true});
    };
    const leave=()=>{if (!pinned) {clearTimeout(leaveTimer);leaveTimer=setTimeout(()=>hide(),120);}};
    document.addEventListener('pointerover',event=>{
      if (event.pointerType==='touch' || hoverDismissed) return;
      const value=event.target.closest('[data-row-comparison]');
      if (value) show(value);
    },{signal});
    document.addEventListener('pointermove',event=>{
      if (event.pointerType==='touch') return;
      // Closing a dialog may reveal a different row under a stationary mouse.
      // Resume hover only when the pointer actually moves, not on DOM changes.
      if (hoverDismissed && (event.clientX!==pointerX || event.clientY!==pointerY)) {
        hoverDismissed=false;
        const value=event.target.closest('[data-row-comparison]');
        if (value) show(value);
      }
      pointerX=event.clientX;pointerY=event.clientY;
    },{signal,passive:true});
    document.addEventListener('pointerout',event=>{
      if (selected?.contains(event.target) && !selected.contains(event.relatedTarget) && !tooltip.contains(event.relatedTarget)) leave();
    },{signal});
    tooltip.addEventListener('pointerenter',()=>clearTimeout(leaveTimer),{signal});
    tooltip.addEventListener('pointerleave',leave,{signal});
    document.addEventListener('focusin',event=>{
      if (tooltip.contains(event.target)) return;
      const value=event.target.closest('[data-row-comparison]')||event.target.closest('.data-row')?.querySelector('[data-row-comparison]');
      if (value) show(value);else if (!pinned) hide();
    },{signal});
    document.addEventListener('click',event=>{
      const value=event.target.closest('[data-row-comparison]');
      if (!value || event.button!==0 || event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) return;
      event.preventDefault();show(value,true);
    },{signal,capture:true});
    document.addEventListener('pointerdown',event=>{
      if (selected && !selected.contains(event.target) && !tooltip.contains(event.target)) hide();
    },{signal});
    document.addEventListener('keydown',event=>{
      if (event.key==='Escape' && selected && !tooltip.hidden) {event.preventDefault();event.stopImmediatePropagation();hide(true);return;}
      if (['Enter',' '].includes(event.key) && event.target.matches('[data-row-comparison][role=button]')) {event.preventDefault();show(event.target,true);}
    },{signal,capture:true});
    close.addEventListener('click',()=>hide(true),{signal});
    window.addEventListener('resize',place,{signal});
    window.addEventListener('scroll',place,{signal,capture:true});
    signal.addEventListener('abort',()=>clearTimeout(leaveTimer),{once:true});
  }

  function initializeMaps(signal) {
    for (const canvas of document.querySelectorAll('.map-canvas')) {
      const tooltip = canvas.querySelector('.map-tooltip');
      const outline = canvas.querySelector('.map-outline');
      const hide = () => { tooltip.hidden = true; outline.setAttribute('hidden',''); };
      function show(shape, x, y) {
        if (!shape) { hide(); return; }
        outline.setAttribute('href',shape.getAttribute('href'));
        outline.removeAttribute('hidden');
        const country = shape.closest('a[data-country]');
        tooltip.hidden = !country;
        if (!country) return;
        tooltip.querySelector('strong').textContent = country.dataset.name;
        tooltip.querySelector('b').textContent = country.dataset.value;
        tooltip.querySelector('span').textContent = country.dataset.unit;
        const bounds = canvas.getBoundingClientRect();
        tooltip.style.left = `${Math.max(0,Math.min(x-bounds.left+8,bounds.width-tooltip.offsetWidth))}px`;
        tooltip.style.top = `${Math.max(0,Math.min(y-bounds.top+8,bounds.height-tooltip.offsetHeight))}px`;
      }
      // Native titles remain the fallback until the local tooltip is ready.
      for (const title of canvas.querySelectorAll('a[data-country]>title')) title.remove();
      canvas.addEventListener('pointermove',event=>{
        if (event.pointerType !== 'touch') show(event.target.closest('.map-country'),event.clientX,event.clientY);
      });
      canvas.addEventListener('pointerleave',hide);
      canvas.addEventListener('focusin',event=>{
        const shape = event.target.closest('a[data-country]')?.querySelector('.map-country');
        if (shape) { const box = shape.getBoundingClientRect(); show(shape,box.x+box.width/2,box.y+box.height/2); }
      });
      canvas.addEventListener('focusout',hide);
      canvas.addEventListener('keydown',event=>{
        if(event.key==='Escape'&&!tooltip.hidden){event.preventDefault();hide();}
      });
      window.addEventListener('resize',hide,{signal});
    }
  }
  function initializeCalendars(signal) {
    // Calendar arithmetic uses UTC dates, while the reporting timezone's bounds
    // come from Rust. Browser timezone and DST must never move a selected day.
    const months=['January','February','March','April','May','June','July','August','September','October','November','December'];
    const dayNumber=value=>Date.parse(`${value}T00:00:00Z`)/86400000;
    const dayDate=day=>new Date(day*86400000);
    const iso=day=>dayDate(day).toISOString().slice(0,10);
    const monthNumber=day=>dayDate(day).getUTCFullYear()*12+dayDate(day).getUTCMonth();
    const firstDay=month=>{const date=new Date(0);date.setUTCFullYear(Math.floor(month/12),month%12,1);return date.getTime()/86400000;};
    for(const form of document.querySelectorAll('.custom-range[data-calendar-min]')) {
      const min=dayNumber(form.dataset.calendarMin),max=dayNumber(form.dataset.calendarMax);
      if(!Number.isInteger(min)||!Number.isInteger(max)||min>max) continue;
      const toggle=form.closest('.custom-range-toggle'),menu=form.closest('.dropdown');
      const fields=[...form.querySelectorAll('input[type=date]')],submit=form.querySelector('button');
      const calendar=document.createElement('div');calendar.className='range-calendar';
      calendar.innerHTML='<div class="calendar-heading"><button type="button" class="calendar-prev" aria-label="Previous calendar month"><svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5"><path d="m15 4-8 8 8 8"/></svg></button><div class="calendar-month"><select aria-label="Month"></select><input type="number" aria-label="Year"></div><button type="button" class="calendar-next" aria-label="Next calendar month"><svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5"><path d="m9 4 8 8-8 8"/></svg></button></div><div class="calendar-weekdays">'+['Su','Mo','Tu','We','Th','Fr','Sa'].map(day=>`<span>${day}</span>`).join('')+'</div><div class="calendar-days"></div>';
      form.prepend(calendar);form.dataset.calendarReady='';
      const month=calendar.querySelector('select'),year=calendar.querySelector('input'),grid=calendar.querySelector('.calendar-days');
      const previous=calendar.querySelector('.calendar-prev'),next=calendar.querySelector('.calendar-next');
      year.min=String(dayDate(min).getUTCFullYear());year.max=String(dayDate(max).getUTCFullYear());
      const selected=form.dataset.calendarSelected==='true'?fields.map(field=>dayNumber(field.value)):null;
      let view=monthNumber(max),anchor=null;
      const paint=(end,start=anchor)=>{
        const from=start===null?null:Math.min(start,end),to=start===null?null:Math.max(start,end);
        for(const button of grid.children) {
          const value=Number(button.dataset.day);
          button.toggleAttribute('data-start',value===from);
          button.toggleAttribute('data-end',value===to);
          button.toggleAttribute('data-between',from!==null&&value>from&&value<to);
        }
      };
      const paintSelection=()=>anchor===null&&selected?paint(selected[1],selected[0]):paint(anchor);
      const draw=()=>{
        view=Math.max(monthNumber(min),Math.min(monthNumber(max),view));
        const y=Math.floor(view/12),m=view%12,start=firstDay(view);
        year.value=String(y);month.replaceChildren();
        for(let i=0;i<12;i++) if(y*12+i>=monthNumber(min)&&y*12+i<=monthNumber(max)) month.add(new Option(months[i],String(i)));
        month.value=String(m);previous.disabled=view<=monthNumber(min);next.disabled=view>=monthNumber(max);
        const first=start-dayDate(start).getUTCDay();
        grid.replaceChildren();
        for(let index=0;index<42;index++) {
          const value=first+index,date=dayDate(value),button=document.createElement('button');
          button.type='button';button.dataset.day=String(value);button.dataset.date=iso(value);button.textContent=String(date.getUTCDate());
          button.setAttribute('aria-label',`${months[date.getUTCMonth()]} ${date.getUTCDate()}, ${date.getUTCFullYear()}`);
          button.disabled=value<min||value>max||(anchor!==null&&Math.abs(value-anchor)>3660);
          button.toggleAttribute('data-outside',date.getUTCMonth()!==m);
          button.toggleAttribute('data-today',value===max);grid.append(button);
        }
        paintSelection();
      };
      const open=()=>{anchor=null;view=monthNumber(selected?selected[0]:max);draw();};
      // Populate before the controls become editable. A deferred toggle event
      // can overwrite typing that has already started in the year field.
      toggle.querySelector('summary').addEventListener('click',()=>{if(!toggle.open)open();},{signal});
      menu.addEventListener('toggle',()=>{if(!menu.open)toggle.open=false;},{signal});
      previous.addEventListener('click',()=>{view--;draw();},{signal});
      next.addEventListener('click',()=>{view++;draw();},{signal});
      month.addEventListener('change',()=>{view=Math.floor(view/12)*12+Number(month.value);draw();},{signal});
      const changeYear=()=>{
        const value=Number(year.value);if(!Number.isInteger(value)||!year.value)return;
        const target=Math.max(monthNumber(min),Math.min(monthNumber(max),Math.max(Number(year.min),Math.min(Number(year.max),value))*12+view%12));
        // Enter commits immediately; its later blur must not replace a day
        // between pointerdown and click and silently swallow the selection.
        if(target!==view) {view=target;draw();} else year.value=String(Math.floor(view/12));
      };
      year.addEventListener('change',changeYear,{signal});
      year.addEventListener('keydown',event=>{if(event.key==='Enter'){event.preventDefault();changeYear();}},{signal});
      grid.addEventListener('pointerover',event=>{const button=event.target.closest('button');if(anchor!==null&&button&&!button.disabled)paint(Number(button.dataset.day));},{signal});
      grid.addEventListener('pointerleave',paintSelection,{signal});
      grid.addEventListener('click',event=>{
        const button=event.target.closest('button');if(!button||button.disabled)return;
        const value=Number(button.dataset.day);
        if(anchor===null) {anchor=value;view=monthNumber(value);draw();return;}
        fields[0].value=iso(Math.min(anchor,value));fields[1].value=iso(Math.max(anchor,value));
        form.requestSubmit(submit);
      },{signal});
      if(toggle.open)open();
    }
  }

  function initialize() {
  controls?.abort();
  controls = new AbortController();
  const signal = controls.signal;
  initializeCalendars(signal);
  const menus = [...document.querySelectorAll('details.dropdown')];
  // Center on the actual trigger, then clamp at the viewport edge. Measuring the
  // trigger also handles wrapped mobile controls and the sticky toolbar.
  const positionMenu = menu => {
    if (!menu.open) return;
    const content = menu.querySelector(':scope > .dropdown-content');
    const trigger = menu.querySelector(':scope > summary');
    if (!content || !trigger || !content.offsetParent) return;
    const anchor = trigger.getBoundingClientRect(), parent = content.offsetParent.getBoundingClientRect();
    const left = Math.max(12, Math.min(anchor.left + anchor.width / 2 - content.offsetWidth / 2, innerWidth - content.offsetWidth - 12));
    content.style.left = `${left - parent.left}px`;
    content.style.right = 'auto';
    content.style.top = `${anchor.bottom + 6 - parent.top}px`;
    content.style.maxHeight = `${Math.max(0, Math.min(innerHeight * .75, innerHeight - anchor.bottom - 18))}px`;
    content.setAttribute('data-positioned','');
  };
  const positionMenus = () => menus.forEach(positionMenu);
  const menuSizes = new ResizeObserver(positionMenus);
  // Native toggle events are queued tasks; old inline coordinates can otherwise
  // flash after resizing a closed menu. Observe the open attribute before paint.
  const menuState = new MutationObserver(positionMenus);
  for (const menu of menus) {
    const content = menu.querySelector(':scope > .dropdown-content');
    if (content) menuSizes.observe(content);
    menuState.observe(menu, {attributes:true,attributeFilter:['open']});
  }
  signal.addEventListener('abort', () => { menuSizes.disconnect(); menuState.disconnect(); }, {once:true});
  window.addEventListener('resize', positionMenus, {signal});
  document.addEventListener('scroll', positionMenus, {capture:true,signal});
  positionMenus();
  const reportSearch = document.querySelector('.report-search input[type=search]');
  reportSearch?.setAttribute('aria-keyshortcuts','/');
  const shortcuts = new Map([...document.querySelectorAll('[data-shortcut]')].filter(element=>element.dataset.shortcut).map(element=>[element.dataset.shortcut,element]));
  for (const [key, element] of shortcuts) element.setAttribute('aria-keyshortcuts', ({arrowleft:'ArrowLeft',arrowright:'ArrowRight'})[key] || key.toUpperCase());
  document.addEventListener('keydown', event => {
    if (event.defaultPrevented || event.repeat || event.metaKey || event.ctrlKey || event.altKey || event.target.closest('.row-tooltip,input,textarea,select,[contenteditable]:not([contenteditable=false])')) return;
    if (event.key === '/' && reportSearch) { event.preventDefault(); reportSearch.focus(); reportSearch.select(); return; }
    if (document.querySelector('.report-overlay')) return;
    if (['ArrowLeft','ArrowRight'].includes(event.key) && (event.shiftKey || event.target.closest('.chart') || document.querySelector('.filter-picker,main details[open]'))) return;
    const target = shortcuts.get(event.key.toLowerCase());
    if (!target) return;
    event.preventDefault();
    if (target.matches('summary')) {
      for (let parent = target.parentElement.parentElement; parent; parent = parent.parentElement) if (parent.matches('details')) parent.open = true;
      if (!target.parentElement.open) target.click();
      (target.parentElement.querySelector('.range-calendar select,input:not([type=hidden])') || target).focus();
    } else target.click();
  }, {signal});
  document.addEventListener('pointerdown', event => {
    for (const menu of menus) if (menu.open && !menu.contains(event.target)) menu.open = false;
  }, {signal});
  document.addEventListener('keydown', event => {
    if (event.key !== 'Escape' || event.defaultPrevented) return;
    for (const menu of menus) {
      if (!menu.open) continue;
      const focused = menu.contains(document.activeElement);
      menu.open = false;
      event.preventDefault();
      if (focused) menu.querySelector('summary').focus();
    }
  }, {signal});
  for (const plot of document.querySelectorAll('.chart-plot')) {
    const chart = plot.querySelector('.chart');
    const points = [...chart.querySelectorAll('.chart-hit')];
    const tooltip = plot.querySelector('.chart-tooltip');
    const cursor = chart.querySelector('.chart-cursor');
    if (!points.length || !tooltip || !cursor) continue;

    // The server supplies site-local wall times and timezone abbreviations.
    // Only the clock convention comes from the browser, never its timezone.
    let twelveHours = false;
    try { twelveHours = new Intl.DateTimeFormat(navigator.language, {hour:'numeric'}).resolvedOptions().hour12; } catch {}
    function formatClock(clock) {
      if (!twelveHours || !/^\d{2}:\d{2}$/.test(clock)) return clock;
      const [hour, minute] = clock.split(':').map(Number);
      const minutes = chart.dataset.interval === 'minute' || minute !== 0 ? `:${String(minute).padStart(2,'0')}` : '';
      return `${hour % 12 || 12}${minutes}${hour >= 12 ? 'pm' : 'am'}`;
    }
    for (const point of points) {
      if (point.dataset.clockLocalized) continue;
      const {clock, previousClock} = point.dataset;
      if (clock) point.dataset.date = point.dataset.date.replace(clock, formatClock(clock));
      if (previousClock) point.dataset.previousDate = point.dataset.previousDate.replace(previousClock, formatClock(previousClock));
      const label = point.getAttribute('aria-label');
      point.setAttribute('aria-label', label.replace(/\b\d{2}:\d{2}\b/g, value =>
        value === clock || value === previousClock ? formatClock(value) : value));
      point.dataset.clockLocalized = 'true';
    }
    for (const label of plot.querySelectorAll('.chart-labels [data-clock]')) {
      if (label.dataset.clock) label.textContent = formatClock(label.dataset.clock);
    }

    const line = cursor.querySelector('line');
    const markers = [...cursor.querySelectorAll('path')];
    const current = tooltip.querySelector('.chart-tooltip-current');
    const previous = tooltip.querySelector('.chart-tooltip-previous');
    const change = tooltip.querySelector('small');
    const zoom = tooltip.querySelector('.chart-tooltip-zoom');
    const hint = tooltip.querySelector('footer');
    let selected = null, tabStop = points[0], mode = '';

    function hide() {
      tooltip.hidden = true;
      cursor.setAttribute('hidden', '');
      selected = null;
      mode = '';
      tooltip.setAttribute('role','tooltip');
    }

    function place() {
      if (!selected) return;
      const bounds = chart.getBoundingClientRect();
      const x = Number(selected.dataset.x) * bounds.width / 1000;
      const width = tooltip.offsetWidth;
      const left = x + width + 12 <= bounds.width ? x + 12 : x - width - 12;
      tooltip.style.left = `${Math.max(0, Math.min(left, bounds.width - width))}px`;
      tooltip.style.top = '0px';
    }

    function show(point, interaction) {
      mode = interaction;
      const pinned = interaction === 'touch' || interaction === 'pinned';
      tooltip.setAttribute('role',pinned ? 'dialog' : 'tooltip');
      tooltip.setAttribute('aria-label','Chart details');
      hint.hidden = pinned || interaction === 'keyboard';
      zoom.hidden = !pinned || !point.hasAttribute('href');
      if (!zoom.hidden) { zoom.href = point.getAttribute('href'); zoom.textContent = `View ${point.dataset.zoom}`; }
      if (selected === point) { place(); return; }
      selected = point;
      tabStop.setAttribute('tabindex', '-1');
      point.setAttribute('tabindex', '0');
      tabStop = point;
      const data = point.dataset;
      // Keep years and the site's clock. Only repeated DST hours need a zone
      // in the compact tooltip; the full labels retain it for every interval.
      const date = (label, clock, ambiguous) => clock && !ambiguous ? label.replace(/ [^ ]+$/, '') : label;
      current.hidden = !data.date;
      current.querySelector('span').textContent = date(data.date, data.clock, data.ambiguousClock);
      current.querySelector('strong').textContent = data.displayValue || data.value;
      current.querySelector('strong').title = data.value;
      previous.hidden = !data.previousDate;
      previous.querySelector('span').textContent = date(data.previousDate, data.previousClock, data.previousAmbiguousClock);
      previous.querySelector('strong').textContent = data.previousDisplayValue || data.previousValue;
      previous.querySelector('strong').title = data.previousValue;
      change.hidden = !data.change;
      change.textContent = data.change;
      change.className = data.tone;
      line.setAttribute('x1', data.x);
      line.setAttribute('x2', data.x);
      [data.y, data.previousY].forEach((y, index) => {
        markers[index].setAttribute('d', y ? `M${data.x} ${y}h0` : '');
      });
      cursor.removeAttribute('hidden');
      tooltip.hidden = false;
      place();
    }

    function pointAt(event) {
      const point = event.target.closest('.chart-hit');
      return point && chart.contains(point) ? point : null;
    }

    points.forEach((point, index) => {
      point.setAttribute('tabindex', index === 0 ? '0' : '-1');
      // Avoid a second browser tooltip. aria-label retains the complete fallback.
      point.querySelector('title')?.remove();
    });
    chart.dataset.interactive = 'true';
    chart.setAttribute('aria-description', 'Use Left and Right to inspect intervals, Home and End to jump, Enter to open a linked period, and Escape to close details.');
    plot.addEventListener('pointermove', event => {
      if (event.pointerType === 'touch' || mode === 'pinned') return;
      const point = pointAt(event);
      if (point) show(point, 'pointer');
      else if (!tooltip.contains(event.target) && mode === 'pointer') hide();
    });
    plot.addEventListener('pointerleave', () => {
      if (mode === 'pointer') hide();
    });
    chart.addEventListener('pointerup', event => {
      if (event.button !== 0) return;
      const point = pointAt(event);
      if (point) show(point, event.pointerType === 'touch' ? 'touch' : 'pointer');
    });
    chart.addEventListener('click', event => {
      const point = pointAt(event);
      if (point && (event.pointerType === 'touch' || mode === 'touch')) {
        event.preventDefault(); show(point,'touch');
      }
    });
    chart.addEventListener('contextmenu', event => {
      const point = pointAt(event);
      if (point) { event.preventDefault(); show(point,'pinned'); }
    });
    chart.addEventListener('focusin', event => {
      const point = pointAt(event);
      if (point) show(point, 'keyboard');
    });
    plot.addEventListener('focusout', event => {
      if (!plot.contains(event.relatedTarget)) hide();
    });
    chart.addEventListener('keydown', event => {
      const point = pointAt(event);
      if (!point) return;
      const index = points.indexOf(point);
      const next = {
        ArrowLeft: Math.max(0, index - 1),
        ArrowRight: Math.min(points.length - 1, index + 1),
        Home: 0,
        End: points.length - 1,
      }[event.key];
      if (next !== undefined) {
        event.preventDefault();
        points[next].focus({preventScroll: true});
        show(points[next], 'keyboard');
      }
    });
    document.addEventListener('keydown', event => {
      if (event.key === 'Escape' && selected) {
        if (tooltip.contains(document.activeElement)) selected.focus({preventScroll:true});
        hide(); event.preventDefault();
      }
    }, {signal});
    document.addEventListener('pointerdown', event => {
      if (!plot.contains(event.target)) hide();
    }, {signal});
    window.addEventListener('resize', place, {signal});
  }
  initializeRowComparisons(signal);
  initializeMaps(signal);
  document.addEventListener('keydown', event => {
    if (event.key !== 'Escape' || event.defaultPrevented) return;
    const close = document.querySelector('.close-report');
    if (close) { event.preventDefault(); close.click(); return; }
    const done = document.querySelector('.filter-done');
    if (done) { event.preventDefault(); follow(done); }
    else if (!event.target.closest('input,textarea,select')) {
      const clear = document.querySelector('.clear-filters');
      if (clear) { event.preventDefault(); follow(clear); }
    }
  }, {signal});
  document.addEventListener('pointerdown', event => {
    const done = document.querySelector('.filter-done');
    if (done && !event.target.closest('.filter,.dropdown,a,button,input,textarea,select')) follow(done);
  }, {signal});
  document.documentElement.dataset.dashboardEnhanced = '';
  savePreferences();
  }

  // Serialize updates so a search or a second filter cannot lose a pending
  // selection. Unsent changes to the same control collapse to the latest value.
  let currentURL = new URL(location.href), renderedURL = new URL(location.href), running = false, searchTimer, request, refreshRequest;
  const jobs = [];
  function follow(link) {
    enqueue({type:'link', key:link.dataset.filterLink, href:link.href});
  }
  function enqueue(job) {
    refreshRequest?.abort();
    const last = jobs.at(-1);
    if (last && job.type !== 'link' && last.type === job.type && last.key === job.key) jobs[jobs.length-1] = job;
    else jobs.push(job);
    if (!running) drain();
  }
  function focusState() {
    const element = document.activeElement;
    if (!element?.closest('.filter-picker,.report-search')) return null;
    return {id:element.id, value:element.type === 'checkbox' ? element.value : null,
      start:element.selectionStart, end:element.selectionEnd};
  }
  function render(html, focus) {
    const active = document.activeElement;
    const typing = ['filter-search','report-search'].includes(active?.id) ? {id:active.id,value:active.value} : null;
    const doc = new DOMParser().parseFromString(html, 'text/html');
    const main = doc.querySelector('main.container');
    if (!main) throw new Error('Missing dashboard');
    main.querySelectorAll('script').forEach(script=>script.remove());
    document.querySelector('main.container').replaceWith(main);
    document.title = doc.title;
    renderedURL = new URL(currentURL);
    initialize();
    if (typing && main.querySelector(`#${typing.id}`)) main.querySelector(`#${typing.id}`).value = typing.value;
    let target;
    if (focus?.value !== null && focus?.value !== undefined) {
      target = [...main.querySelectorAll('.filter-values input[type=checkbox]')].find(input=>input.value===focus.value);
    } else if (focus?.id) target = document.getElementById(focus.id);
    if (target) {
      target.focus({preventScroll:true});
      if (focus.start !== null && typeof target.setSelectionRange === 'function') target.setSelectionRange(focus.start,focus.end);
    }
  }
  function navigation(job) {
    const target = new URL(job.href), before = new URL(job.base);
    if (target.origin !== currentURL.origin || !target.pathname.startsWith(currentURL.pathname + '/') && target.pathname !== currentURL.pathname || !target.search) return target;
    const dimensions=['page','entry','exit','source','referrer','channel','utm_medium','utm_source','utm_campaign','utm_content','utm_term','country','region','city','browser','browser_version','os','os_version','device'];
    const filters = url => {
      const result=new Map(dimensions.filter(key=>url.searchParams.has(key)).map(key=>[key,{key,op:'is',values:[url.searchParams.get(key)]}]));
      for(const filter of JSON.parse(url.searchParams.get('filters')||'[]')) result.set(filter.key,filter);
      return result;
    };
    const was=filters(before), next=filters(target), merged=filters(currentURL);
    for(const key of dimensions) if(JSON.stringify(was.get(key))!==JSON.stringify(next.get(key))) {
      if(next.has(key)) merged.set(key,next.get(key)); else merged.delete(key);
      for(const child of ({country:['region','city'],region:['city'],browser:['browser_version'],os:['os_version']}[key]||[])) merged.delete(child);
    }
    const params=new URLSearchParams(currentURL.search);
    for(const key of new Set([...before.searchParams.keys(),...target.searchParams.keys()])) {
      if(dimensions.includes(key)||key==='filters') continue;
      if(JSON.stringify(before.searchParams.getAll(key))!==JSON.stringify(target.searchParams.getAll(key))) {
        params.delete(key);for(const value of target.searchParams.getAll(key)) params.append(key,value);
      }
    }
    // Closing a report also discards a search that finished after its close
    // link was rendered. It must not leak into the dashboard underneath.
    if (before.searchParams.has('detail') && !target.searchParams.has('detail')) {
      for (const key of ['detail','search','sort','order','offset']) params.delete(key);
    }
    for(const key of dimensions) params.delete(key);
    params.delete('filters');if(merged.size) params.set('filters',JSON.stringify([...merged.values()]));
    if(!target.searchParams.has('filter')) for(const key of ['filter','filter_mode','filter_op','filter_search','filter_offset']) params.delete(key);
    target.search=params.toString();return target;
  }
  const isRealtime = () => document.querySelector('.dashboard-toolbar')?.dataset.realtime === 'true';
  function liveStatus(text) {
    const status = document.querySelector('.live-status');
    if (!status) return;
    status.textContent = text;
    status.toggleAttribute('data-stale',text !== 'Live');
  }
  function editingReport() {
    const search = document.querySelector('.report-search input[type=search]');
    return document.querySelector('.filter-picker,.report-overlay,main details[open],.chart-tooltip:not([hidden]),.map-tooltip:not([hidden])') || document.activeElement?.closest('main a,main button,main input,main select,main textarea,main summary,main .chart-hit') || search && search.value !== search.defaultValue;
  }
  async function refreshRealtime() {
    if (!isRealtime() || document.hidden || refreshRequest || running || jobs.length) return;
    // Never replace a form, focused control, open menu or chart details while
    // the owner is using it. The next tick resumes after the interaction ends.
    if (editingReport()) { liveStatus('Paused'); return; }
    const before = currentURL.href, controller = new AbortController();
    refreshRequest = controller;
    const timeout = setTimeout(()=>controller.abort('timeout'),15000);
    try {
      const response = await fetch(before,{credentials:'same-origin',cache:'no-store',signal:controller.signal});
      if (!response.ok) throw new Error('Refresh failed');
      const target = new URL(response.url);
      if (controller.signal.aborted || before !== currentURL.href) return;
      if (target.origin !== location.origin) throw new Error('Invalid refresh URL');
      if (target.pathname === '/login') { location.assign(target); return; }
      const html = await response.text();
      if (controller.signal.aborted || running || jobs.length || before !== currentURL.href || document.hidden || editingReport()) return;
      if (target.origin !== location.origin || target.pathname !== currentURL.pathname) throw new Error('Invalid refresh URL');
      render(html,null);
    } catch {
      if (!controller.signal.aborted || controller.signal.reason === 'timeout') liveStatus('Reconnecting…');
    } finally {
      clearTimeout(timeout);
      if (refreshRequest === controller) refreshRequest = null;
    }
  }
  async function drain() {
    running = true;
    let target;
    try {
      while (jobs.length) {
        const job = jobs.shift();
        if (job.type === 'navigation') { jobs.length=0; location.assign(navigation(job)); return; }
        target = new URL(currentURL);
        let focus = focusState();
        if (job.type === 'link') {
          const fresh = [...document.querySelectorAll('a[data-filter-link]')].find(link=>link.dataset.filterLink===job.key);
          target = new URL(fresh?.href || job.href);
          if (/^(open:|values:)/.test(job.key)) focus = {id:'filter-search',value:null,start:0,end:0};
        } else if (job.type === 'report-search') {
          if (job.value) target.searchParams.set('search',job.value); else target.searchParams.delete('search');
          target.searchParams.delete('offset');
        } else if (job.type === 'search') {
          target.searchParams.set('filter',job.key);
          target.searchParams.set('filter_search',job.value);
          target.searchParams.delete('filter_offset');
          focus = {id:'filter-search',value:null,start:job.start,end:job.end};
        } else {
          target.pathname = target.pathname.replace(/\/filter$/, '') + '/filter';
          target.searchParams.set('filter',job.key);
          target.searchParams.set('filter_op',job.op);
          target.searchParams.delete('value');
          target.searchParams.delete('term');
          for (const value of job.values) target.searchParams.append('value',value);
          if (job.term) target.searchParams.set('term',job.term);
          if (job.operator) focus = {id:'filter-search',value:null,start:0,end:0};
        }
        if (target.origin !== location.origin) throw new Error('Invalid filter URL');
        document.querySelector('main.container').setAttribute('aria-busy','true');
        request = new AbortController();
        const response = await fetch(target, {credentials:'same-origin', signal:request.signal});
        if (!response.ok) throw new Error('Filter request failed');
        const html = await response.text();
        currentURL = new URL(response.url);
        if (currentURL.pathname === '/login') { location.assign(currentURL); return; }
        history[['search','report-search'].includes(job.type) ? 'replaceState' : 'pushState'](null,'',currentURL);
        // Keep the live checkboxes/search text while queued changes are sent.
        // Links need freshly rendered destinations, including parent removal.
        if (!jobs.length || jobs[0].type === 'link' || jobs[0].type === 'navigation') {
          if (job.type === 'apply' && !job.operator || job.type === 'report-search') focus = focusState();
          render(html, focus);
        }
      }
    } catch (error) {
      if (error.name !== 'AbortError' && target) location.assign(target);
    } finally {
      running = false;
      document.querySelector('main.container')?.removeAttribute('aria-busy');
    }
  }
  function apply(form, submitter) {
    const data = new FormData(form, submitter);
    enqueue({type:'apply', key:data.get('filter'), op:data.get('filter_op'),
      values:data.getAll('value'), term:data.get('term'), operator:form.classList.contains('operator-options')});
  }
  document.addEventListener('submit', event => {
    refreshRequest?.abort();
    const form = event.target;
    if (form.matches('.filter-apply')) { event.preventDefault(); apply(form,event.submitter); }
    else if (form.matches('.filter-search')) { event.preventDefault(); clearTimeout(searchTimer); search(form.querySelector('[name=filter_search]')); }
    else if (form.matches('.report-search')) { event.preventDefault(); clearTimeout(searchTimer); search(form.querySelector('[name=search]')); }
    else if (running && form.method.toLowerCase() === 'get' && form.closest('main.container') && !form.target) {
      event.preventDefault(); clearTimeout(searchTimer);
      const target = new URL(form.action || location.href);
      target.search = new URLSearchParams(new FormData(form,event.submitter));
      enqueue({type:'navigation',href:target.href,base:renderedURL.href});
    }
  });
  document.addEventListener('change', event => {
    if (event.target.matches('.filter-values input[type=checkbox]')) apply(event.target.form);
  });
  function search(input) {
    if (input.form.matches('.report-search')) { enqueue({type:'report-search',value:input.value}); return; }
    const form = new FormData(input.form);
    enqueue({type:'search',key:form.get('filter'),value:input.value,start:input.selectionStart,end:input.selectionEnd});
  }
  document.addEventListener('input', event => {
    if (!event.target.matches('.filter-search input,.report-search input')) return;
    clearTimeout(searchTimer);
    searchTimer = setTimeout(()=>search(event.target),200);
  });
  document.addEventListener('click', event => {
    const link = event.target.closest('a');
    if (!link || event.defaultPrevented || event.button !== 0 || event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) return;
    rememberChoice(link);
    refreshRequest?.abort();
    clearTimeout(searchTimer);
    if (!link.hasAttribute('data-filter-link')) {
      if (running && !link.getAttribute('target') && !link.hasAttribute('download')) {
        event.preventDefault();clearTimeout(searchTimer);
        enqueue({type:'navigation',href:new URL(link.getAttribute('href'),location.href).href,base:renderedURL.href});
      }
      return;
    }
    event.preventDefault();
    clearTimeout(searchTimer);
    follow(link);
  });
  window.addEventListener('popstate', () => { jobs.length=0; request?.abort(); refreshRequest?.abort(); location.reload(); });
  document.addEventListener('visibilitychange', () => {
    if (document.hidden) refreshRequest?.abort(); else refreshRealtime();
  });
  window.addEventListener('pagehide', () => refreshRequest?.abort());
  setInterval(refreshRealtime,30000);
  initialize();
})();
