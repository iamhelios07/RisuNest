<script lang="ts">
    import ListPager from "src/lib/UI/GUI/ListPager.svelte"
    import { loreListWindow, loreDropIndex, groupLoreFolders } from "src/ts/gui/loreListWindow"
    import { type loreBook } from "src/ts/storage/database.svelte";
    import { DBState } from 'src/ts/stores.svelte';
    import LoreBookData from "./LoreBookData.svelte";
    import { selectedCharID } from "src/ts/stores.svelte";
    import Sortable from 'sortablejs/modular/sortable.core.esm.js';
    import { onDestroy, onMount } from "svelte";
    import { sortableOptions } from "src/ts/util";
    import { v4 } from "uuid";

    interface Props {
        globalMode?: boolean;
        submenu?: number;
        lorePlus?: boolean;
        externalLoreBooks?: loreBook[];
        showFolder?: string
        dragGroup?: string
    }

    let { globalMode = false, submenu = 0, lorePlus = false, externalLoreBooks = null, showFolder = '', dragGroup = 'a' + v4() }: Props = $props();
    let page = $state(0)
    const currentItems = $derived(externalLoreBooks ?? (globalMode
        ? DBState.db.loreBook[DBState.db.loreBookPage]?.data ?? []
        : submenu === 1
            ? DBState.db.characters[$selectedCharID]?.chats[DBState.db.characters[$selectedCharID].chatPage]?.localLore ?? []
            : DBState.db.characters[$selectedCharID]?.globalLore ?? []))
    const windowed = $derived(loreListWindow(currentItems, showFolder, page))
    const pageRows = $derived(windowed.rows)
    let stb: Sortable = null
    let ele: HTMLDivElement = $state()
    const idgroup = $derived(dragGroup)
    let originalNextSibling: Node | null = null

    const createStb = () => {
        stb = Sortable.create(ele, {
            ...sortableOptions,
            group: idgroup,
            draggable: '> [data-risu-idx]',
            swapThreshold: 0.9,
            preventOnFilter: false,
            animation: 150,
            chosenClass: "risu-chosen-item",
            ghostClass: "risu-ghost-item",
            onMove: (event) => {
                const moved = currentItems[Number(event.dragged.getAttribute('data-risu-idx'))]
                if (moved?.mode === 'folder' && event.from !== event.to) return false
                return sortableOptions.onMove(event)
            },
            onStart: (event) => {
                originalNextSibling = event.item.nextSibling
            },
            onEnd: (event) => {
                const indexOfRow = (node: Element | null): number | null => {
                    const value = node?.getAttribute('data-risu-idx')
                    return value === null || value === undefined ? null : Number(value)
                }
                const source = indexOfRow(event.item)
                const next = indexOfRow(event.item.nextElementSibling)
                const previous = indexOfRow(event.item.previousElementSibling)
                // Restore Svelte's DOM before changing either folder's keyed list.
                event.from.insertBefore(event.item, originalNextSibling)
                if (source === null || (event.from === event.to && event.oldIndex === event.newIndex)) return

                const moved = currentItems[source]
                if (!moved) return
                const targetFolder = event.to.getAttribute('data-show-folder') || ''
                if (moved.mode === 'folder' && targetFolder) return
                const destination = loreDropIndex(source, next, previous, currentItems.length)
                if (targetFolder) moved.folder = targetFolder
                else delete moved.folder
                const reordered = [...currentItems]
                reordered.splice(source, 1)
                reordered.splice(destination, 0, moved)
                currentItems.splice(0, currentItems.length, ...groupLoreFolders(reordered))
            },
        })
    }

    onMount(createStb)

    let openedDetails = 0  // Count only lorebook details (for drag deactivation)
    let openedRefs = $state(new Set()) // Track both folders + lorebooks (for UI state)
    
    // Derived state to calculate number of open folders
    let openFolders = $derived(() => {
        let count = 0
        for (const ref of openedRefs) {
            if (ref && typeof ref === 'object' && 'mode' in ref && ref.mode === 'folder') {
                count++
            }
        }
        return count
    })
    
    const onOpen = (isDetail: boolean = true, bookRef?: any) => {
        if (isDetail) {
            // Disable drag only when lorebook detail opens
            openedDetails += 1
            if(stb){
                try {
                    stb.destroy()
                } catch (error) {}
            }
    }
        if (bookRef) {
            openedRefs.add(bookRef)
            openedRefs = new Set(openedRefs) // Trigger reactivity
        }
    }
    const onClose = (isDetail: boolean = true, bookRef?: any) => {
        if (isDetail) {
            // Consider reactivating drag only when lorebook detail closes
            openedDetails -= 1
            if(openedDetails === 0){
                createStb()
            }
        }
        if (bookRef) {
            openedRefs.delete(bookRef)
            openedRefs = new Set(openedRefs) // Trigger reactivity
        }
    }

    onDestroy(() => {
        if(stb){
            try {
                stb.destroy()
            } catch (error) {  }
        }
    })
</script>

<ListPager bind:page total={windowed.total} disabled={openedRefs.size > 0} />
    <div class="border-solid border-selected p-2 flex flex-col border-1 rounded-md" 
         bind:this={ele} 
         data-show-folder={showFolder || ''}>
        {#if globalMode}
            <!--
                This was a place for global lorebooks, but it was removed :)
            -->
        {:else if externalLoreBooks}
            {@const lastVisibleItem = pageRows.at(-1)?.book}
            {#if externalLoreBooks.length === 0}
                <span class="text-textcolor2">No Lorebook</span>
            {:else}
                {#each pageRows as { book, i } (book)}
                    {#if (!showFolder && !book.folder) || (showFolder === book.folder)}
                        <LoreBookData idgroup={idgroup} bind:value={externalLoreBooks[i]} idx={i} 
                        isOpen={openedRefs.has(book)}
                        openFolders={openFolders()}
                        isLastInContainer={book === lastVisibleItem}
                        onRemove={() => {
                            if (openedRefs.has(book)) onClose(book.mode !== 'folder', book)
                            
                            let lore = externalLoreBooks
                            
                            // When deleting a folder, also delete all items that belong to that folder
                            if (book.mode === 'folder') {
                                // Close items belonging to the folder if they are open
                                lore.forEach(item => {
                                    if (item.folder === book.key && openedRefs.has(item)) {
                                        onClose(true, item)
                                    }
                                })
                                
                                // Filter out the folder and all items belonging to it
                                lore = lore.filter(item => 
                                    item !== book && item.folder !== book.key
                                )
                            } else {
                                // Delete regular item
                                lore.splice(i, 1)
                            }
                            
                            if (lore !== externalLoreBooks) {
                                externalLoreBooks.splice(0, externalLoreBooks.length, ...lore)
                            }
                        }} 
                        onOpen={(isDetail = true) => onOpen(isDetail, book)}
                        onClose={(isDetail = true) => onClose(isDetail, book)}
                        bind:externalLoreBooks={externalLoreBooks} />
                    {/if}
                {/each}
            {/if}
        {:else if submenu === 0}
            {@const lastVisibleItem = pageRows.at(-1)?.book}
            {#if DBState.db.characters[$selectedCharID].globalLore.length === 0}
                <span class="text-textcolor2">No Lorebook</span>
            {:else}
                {#each pageRows as { book, i } (book)}
                    {#if (!showFolder && !book.folder) || (showFolder === book.folder)}
                        <LoreBookData idgroup={idgroup} bind:value={DBState.db.characters[$selectedCharID].globalLore[i]} idx={i} 
                        isOpen={openedRefs.has(book)}
                        openFolders={openFolders()}
                        isLastInContainer={book === lastVisibleItem}
                        onRemove={() => {
                            if (openedRefs.has(book)) onClose(book.mode !== 'folder', book)
                            
                            let lore  = DBState.db.characters[$selectedCharID].globalLore
                            
                            // When deleting a folder, also delete all items that belong to that folder
                            if (book.mode === 'folder') {
                                // Close items belonging to the folder if they are open
                                lore.forEach(item => {
                                    if (item.folder === book.key && openedRefs.has(item)) {
                                        onClose(true, item)
                                    }
                                })
                                
                                // Filter out the folder and all items belonging to it
                                lore = lore.filter(item => 
                                    item !== book && item.folder !== book.key
                                )
                            } else {
                                // Delete regular item
                                lore.splice(i, 1)
                            }
                            
                            DBState.db.characters[$selectedCharID].globalLore = lore
                        }} 
                        onOpen={(isDetail = true) => onOpen(isDetail, book)}
                        onClose={(isDetail = true) => onClose(isDetail, book)}
                        lorePlus={lorePlus} bind:externalLoreBooks={DBState.db.characters[$selectedCharID].globalLore}/>
                    {/if}
                {/each}
            {/if}
        {:else if submenu === 1}
            {@const lastVisibleItem = pageRows.at(-1)?.book}
            {#if DBState.db.characters[$selectedCharID].chats[DBState.db.characters[$selectedCharID].chatPage].localLore.length === 0}
                <span class="text-textcolor2">No Lorebook</span>
            {:else}
                {#each pageRows as { book, i } (book)}
                    {#if (!showFolder && !book.folder) || (showFolder === book.folder)}
                        <LoreBookData idgroup={idgroup} bind:value={DBState.db.characters[$selectedCharID].chats[DBState.db.characters[$selectedCharID].chatPage].localLore[i]} idx={i} 
                        isOpen={openedRefs.has(book)}
                        openFolders={openFolders()}
                        isLastInContainer={book === lastVisibleItem}
                        onRemove={() => {
                            if (openedRefs.has(book)) onClose(book.mode !== 'folder', book)
                            
                            let lore  = DBState.db.characters[$selectedCharID].chats[DBState.db.characters[$selectedCharID].chatPage].localLore
                            
                            // When deleting a folder, also delete all items that belong to that folder
                            if (book.mode === 'folder') {
                                // Close items belonging to the folder if they are open
                                lore.forEach(item => {
                                    if (item.folder === book.key && openedRefs.has(item)) {
                                        onClose(true, item)
                                    }
                                })
                                
                                // Filter out the folder and all items belonging to it
                                lore = lore.filter(item => 
                                    item !== book && item.folder !== book.key
                                )
                            } else {
                                // Delete regular item
                                lore.splice(i, 1)
                            }
                            
                            DBState.db.characters[$selectedCharID].chats[DBState.db.characters[$selectedCharID].chatPage].localLore = lore
                        }} 
                        onOpen={(isDetail = true) => onOpen(isDetail, book)}
                        onClose={(isDetail = true) => onClose(isDetail, book)}
                        lorePlus={lorePlus} bind:externalLoreBooks={DBState.db.characters[$selectedCharID].chats[DBState.db.characters[$selectedCharID].chatPage].localLore}/>
                    {/if}
                {/each}
            {/if}
        {/if}
    </div>
