import type { KeyboardEvent } from 'react'

/** Enter in a picker's search box picks the highlighted item; it must never
 *  fall through to the browser's implicit form submission. */
export function keepComboboxEnter(e: KeyboardEvent<HTMLFormElement>) {
  if (e.key === 'Enter' && e.target instanceof HTMLElement && e.target.getAttribute('role') === 'combobox') {
    e.preventDefault()
  }
}
