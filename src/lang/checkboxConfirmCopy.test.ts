import { describe, expect, it } from 'vitest'
import { languageChinese } from './cn'
import { languageGerman } from './de'
import { languageEnglish } from './en'
import { languageSpanish } from './es'
import { languageKorean } from './ko'
import { languageVietnamese } from './vi'
import { languageChineseTraditional } from './zh-Hant'

function expectStatement(text: string) {
    expect(text.trim().endsWith('?'), text).toBe(false)
    for (const question of ['까?', '까요?', 'No를', '정말']) expect(text, text).not.toContain(question)
}

describe('checkbox confirmation copy', () => {
    it('offers later-message deletion as an opt-in and states what removal does', () => {
        expect(languageKorean.checkboxConfirmation.laterMessageDeletion).toBe('이후 메시지도 함께 삭제')
        expect(languageEnglish.checkboxConfirmation.laterMessageDeletion).toBe('Also delete later messages')
        expect(languageKorean.checkboxConfirmation.messageDeletionDescription).toBe(
            '이 메시지만 삭제되며, 이후 메시지도 함께 삭제할 경우 이 메시지 뒤의 모든 메시지가 함께 삭제됩니다.',
        )
        expect(languageEnglish.checkboxConfirmation.messageDeletionDescription).toBe(
            'Only this message is deleted. If you also delete later messages, every message after it is deleted too.',
        )
        expectStatement(languageKorean.checkboxConfirmation.messageDeletionDescription)
        expectStatement(languageEnglish.checkboxConfirmation.messageDeletionDescription)
        expect(languageKorean.remove).toBe('삭제')
        for (const language of [
            languageKorean, languageEnglish, languageChinese, languageGerman, languageSpanish, languageVietnamese, languageChineseTraditional,
        ]) {
            expect(language.checkboxConfirmation as Record<string, unknown>).not.toHaveProperty('onlySelectedMessage')
        }
    })
})
