// Extension de démonstration (ADR-0051 § 8), montée par les tests du panel sous le nom `demo` :
// une section du rail gardée par une permission, une entrée ajoutée aux Utilisateurs, une page.
import { NotebookPen, Users } from '@lucide/vue'

import { definePanelExtension } from '@/shared/extensions'

export default definePanelExtension({
  sections: [
    {
      key: 'demo',
      label: 'ext.demo.nav.title',
      icon: NotebookPen,
      to: '/demo/notes',
      title: 'ext.demo.nav.title',
      permission: 'demo.notes.read',
      items: [{ label: 'ext.demo.nav.notes', icon: NotebookPen, to: '/demo/notes', permission: 'demo.notes.read' }],
    },
  ],
  items: {
    users: [{ label: 'ext.demo.nav.customers', icon: Users, to: '/demo/notes', permission: 'demo.notes.read' }],
  },
  messages: {
    fr: {
      nav: { title: 'Démo', notes: 'Notes', customers: 'Clients de la démo' },
      notes: { title: 'Notes de la démo' },
      errors: { notes: { locked: 'Ces notes sont verrouillées.' } },
    },
    en: {
      nav: { title: 'Demo', notes: 'Notes', customers: 'Demo customers' },
      notes: { title: 'Demo notes' },
      errors: { notes: { locked: 'These notes are locked.' } },
    },
  },
})
