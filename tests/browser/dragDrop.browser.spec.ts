import { test, expect } from './fixture'
import type { Locator, Page } from '@playwright/test'

async function order(page: Page, kind: string) {
    return page.evaluate(kind => {
        const db = (window as any).dragState.db
        const items = kind === 'lore' ? db.characters[0].globalLore
            : kind === 'regex' ? db.globalscript : kind === 'trigger' ? db.characters[0].triggerscript : db.promptTemplate
        return items.map((item: any) => item.comment ?? item.name)
    }, kind)
}
async function open(page: Page, kind: string) {
    const errors: string[] = []
    page.on('pageerror', error => errors.push(error.message))
    await page.goto(`/dragDrop.html?kind=${kind}`)
    await expect.poll(async () => ({ mounted: await page.locator('#drag-fixture').count(), errors })).toEqual({ mounted: 1, errors: [] })
}
async function drag(source: Locator, target: Locator, bottom = false) {
    await source.scrollIntoViewIfNeeded()
    const start = await source.boundingBox()
    const box = await target.boundingBox()
    const mouse = source.page().mouse
    await mouse.move(start!.x + start!.width / 2, start!.y + start!.height / 2)
    await mouse.down()
    await mouse.move(start!.x + start!.width / 2 + 8, start!.y + start!.height / 2, { steps: 3 })
    const x = box!.x + box!.width / 2
    const y = box!.y + (bottom ? box!.height - 3 : 3)
    await mouse.move(x, y, { steps: 8 })
    await mouse.move(x, y)
    await mouse.up()
}

for (const kind of ['regex', 'trigger']) {
    test(`${kind}: delete, edit, and repeatedly reorder with the real Sortable library`, async ({ page }) => {
        const errors: string[] = []
        page.on('pageerror', error => errors.push(error.message))
        await open(page, kind)
        await page.getByRole('button', { name: 'First', exact: true }).locator('..').locator('button').nth(1).click()
        await page.getByRole('button', { name: 'Second', exact: true }).click()
        await page.getByRole('button', { name: 'Second', exact: true }).click()
        const last = page.getByRole('button', { name: 'Last', exact: true })
        const second = page.getByRole('button', { name: 'Second', exact: true })
        await drag(last, second)
        await expect.poll(() => order(page, kind)).toEqual(['Last', 'Second'])
        await drag(second, last)
        await expect.poll(() => order(page, kind)).toEqual(['Second', 'Last'])
        expect(errors).toEqual([])
    })
}

test('lore: reorder around an expanded folder without remounting its editors', async ({ page }) => {
    const errors: string[] = []
    page.on('pageerror', error => errors.push(error.message))
    await open(page, 'lore')
    await page.getByRole('button', { name: 'Folder', exact: true }).click()
    await page.getByRole('button', { name: 'Child', exact: true }).click()
    const input = await page.locator('[data-show-folder="folder"] input').first().elementHandle()
    await drag(page.getByRole('button', { name: 'Last', exact: true }), page.getByRole('button', { name: 'First', exact: true }))
    await expect.poll(() => order(page, 'lore')).toEqual(['Folder', 'Child', 'Last', 'First'])
    expect(await input!.evaluate(node => node.isConnected)).toBe(true)
    expect(errors).toEqual([])
})

test('lore: move into a folder and back to the root', async ({ page }) => {
    const errors: string[] = []
    page.on('pageerror', error => errors.push(error.message))
    await open(page, 'lore')
    await page.getByRole('button', { name: 'Folder', exact: true }).click()
    const first = page.getByRole('button', { name: 'First', exact: true })
    await drag(first, page.getByRole('button', { name: 'Child', exact: true }), true)
    const folderOfFirst = () => page.evaluate(() => (window as any).dragState.db.characters[0].globalLore.find((item: any) => item.comment === 'First').folder)
    await expect.poll(folderOfFirst).toBe('folder')
    await expect(page.locator('[data-show-folder="folder"]').getByRole('button', { name: 'First', exact: true })).toBeVisible()
    await drag(first, page.getByRole('button', { name: 'Last', exact: true }), true)
    await expect.poll(folderOfFirst).toBeUndefined()
    expect(errors).toEqual([])
})

test('prompt: use gaps and card halves without changing the drag source during hover', async ({ page }) => {
    const errors: string[] = []
    page.on('pageerror', error => errors.push(error.message))
    await open(page, 'prompt')
    const header = (name: string) => page.locator('[draggable="true"]').filter({ has: page.locator('span', { hasText: new RegExp(`^${name}$`) }) })
    const first = header('First')
    await first.click()
    const input = await first.locator('..').locator('input').first().elementHandle()
    await drag(first, header('Last'), true)
    await expect.poll(() => order(page, 'prompt')).toEqual(['Second', 'Last', 'First'])
    expect(await input!.evaluate(node => node.isConnected)).toBe(true)
    await drag(header('First'), page.locator('[role="doc-pagebreak"]').first())
    await expect.poll(() => order(page, 'prompt')).toEqual(['First', 'Second', 'Last'])
    expect(errors).toEqual([])
})

test.describe('touch dragging', () => {
    test.use({ hasTouch: true, isMobile: true, viewport: { width: 393, height: 852 } })
    test('reorders regex entries after the touch hold delay', async ({ page, context }) => {
        await open(page, 'regex')
        const source = await page.getByRole('button', { name: 'Last', exact: true }).boundingBox()
        const target = await page.getByRole('button', { name: 'First', exact: true }).boundingBox()
        const cdp = await context.newCDPSession(page)
        const x = source!.x + source!.width / 2
        const y = source!.y + source!.height / 2
        await cdp.send('Input.dispatchTouchEvent', { type: 'touchStart', touchPoints: [{ x, y }] })
        await page.waitForTimeout(350)
        for (let step = 1; step <= 6; step++) {
            await cdp.send('Input.dispatchTouchEvent', { type: 'touchMove', touchPoints: [{
                x, y: y + (target!.y + 3 - y) * step / 6,
            }] })
            await page.waitForTimeout(40)
        }
        await cdp.send('Input.dispatchTouchEvent', { type: 'touchEnd', touchPoints: [] })
        await expect.poll(() => order(page, 'regex')).toEqual(['Last', 'First', 'Second'])
    })
})
