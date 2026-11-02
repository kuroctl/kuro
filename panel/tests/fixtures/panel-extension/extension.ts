// Extension de démonstration (ADR-0051 § 8), montée par les tests du panel sous le nom `demo` :
// une section du rail gardée par une permission, dont le sous-menu a un groupe, une entrée ajoutée
// aux Utilisateurs, trois pages.
import { Archive, NotebookPen, Users } from '@lucide/vue'

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
      items: [
        { label: 'ext.demo.nav.notes', icon: NotebookPen, to: '/demo/notes', permission: 'demo.notes.read' },
        {
          label: 'ext.demo.nav.filing',
          icon: Archive,
          children: [
            { label: 'ext.demo.nav.archive', to: '/demo/notes/archive', permission: 'demo.notes.read' },
            { label: 'ext.demo.nav.secret', to: '/demo/notes/secret', permission: 'demo.secret.read' },
          ],
        },
      ],
    },
  ],
  items: {
    users: [{ label: 'ext.demo.nav.customers', icon: Users, to: '/demo/notes', permission: 'demo.notes.read' }],
  },
  messages: {
    fr: {
      nav: { title: 'Démo', notes: 'Notes', customers: 'Clients de la démo', filing: 'Classement', archive: 'Archives', secret: 'Secret' },
      notes: { title: 'Notes de la démo' },
      archive: { title: 'Archives de la démo' },
      secret: { title: 'Notes secrètes' },
      errors: { notes: { locked: 'Ces notes sont verrouillées.' } },
    },
    en: {
      nav: { title: 'Demo', notes: 'Notes', customers: 'Demo customers', filing: 'Filing', archive: 'Archive', secret: 'Secret' },
      notes: { title: 'Demo notes' },
      archive: { title: 'Demo archive' },
      secret: { title: 'Secret notes' },
      errors: { notes: { locked: 'These notes are locked.' } },
    },
  },
})
