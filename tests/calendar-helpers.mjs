// Select real calendar controls, including a range spanning different years.
export async function selectCalendarDate(page,date,comparison=false) {
  const calendar=page.locator(comparison?'.custom-comparison .range-calendar':'.custom-range:not(.custom-comparison) .range-calendar');
  const [year,month]=date.split('-');
  await calendar.getByLabel('Year',{exact:true}).fill(year);
  await calendar.getByLabel('Year',{exact:true}).press('Enter');
  await calendar.getByLabel('Month',{exact:true}).selectOption(String(Number(month)-1));
  await calendar.locator(`[data-date="${date}"]`).click();
}
export async function selectCalendarRange(page,from,to,comparison=false) {
  await selectCalendarDate(page,from,comparison);
  await selectCalendarDate(page,to,comparison);
  await page.waitForURL(url=>url.searchParams.get(comparison?'compare_from':'from')===from&&url.searchParams.get(comparison?'compare_to':'to')===to);
}
