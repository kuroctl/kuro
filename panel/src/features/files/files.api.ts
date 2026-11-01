// Appels API des fichiers d'un serveur (ADR-0015). Aucune logique ici : les erreurs remontent à
// l'appelant.
import { http } from '@/shared/api/http'
import type { Schemas } from '@/shared/api/types'

const base = (server: string) => `/admin/v1/servers/${server}/files`

export type FileEntry = Schemas['FileEntryDto']
export type FileListing = Schemas['FileListingDto']
export type FileContent = Schemas['FileContentDto']
export type SftpConnection = Schemas['SftpConnectionDto']
export type FileVersion = Schemas['FileVersionDto']

export function useFilesApi() {
  return {
    /** Versions gardées d'un fichier (ADR-0049), de la plus récente à la plus ancienne. */
    history: (server: string, path: string) =>
      http
        .get<Schemas['FileHistoryDto']>(`${base(server)}/history`, { params: { path } })
        .then((response) => response.data),

    version: (server: string, path: string, version: string) =>
      http
        .get<FileContent>(`${base(server)}/history/${version}`, { params: { path } })
        .then((response) => response.data),

    /** Fichiers supprimés d'un dossier dont une version est encore gardée. */
    deleted: (server: string, path: string) =>
      http
        .get<Schemas['DeletedFilesDto']>(`${base(server)}/deleted`, { params: { path } })
        .then((response) => response.data),

    restore: (server: string, path: string, version: string) =>
      http.post<void>(`${base(server)}/history/${version}/restore`, { path }).then(() => undefined),

    /** Coordonnées SFTP du serveur pour le compte connecté (ADR-0047). */
    sftp: (server: string) =>
      http.get<SftpConnection>(`/admin/v1/servers/${server}/sftp`).then((response) => response.data),

    list: (server: string, path: string) =>
      http.get<FileListing>(base(server), { params: { path } }).then((response) => response.data),

    read: (server: string, path: string) =>
      http.get<FileContent>(`${base(server)}/content`, { params: { path } }).then((response) => response.data),

    write: (server: string, body: Schemas['WriteFileRequest']) =>
      http.put<void>(`${base(server)}/content`, body).then(() => undefined),

    mkdir: (server: string, path: string) => http.post<void>(`${base(server)}/directories`, { path }).then(() => undefined),

    rename: (server: string, from: string, to: string) =>
      http.post<void>(`${base(server)}/rename`, { from, to }).then(() => undefined),

    /** Envoi binaire (256 Mio au plus), relayé au nœud par morceaux. */
    upload: (server: string, path: string, file: Blob, progress?: (ratio: number) => void) =>
      http
        .put<Schemas['UploadedFileDto']>(`${base(server)}/upload`, file, {
          params: { path },
          headers: { 'content-type': 'application/octet-stream' },
          onUploadProgress: (event) => progress?.(event.total ? event.loaded / event.total : 0),
        })
        .then((response) => response.data),

    remove: (server: string, path: string) => http.delete<void>(base(server), { params: { path } }).then(() => undefined),
  }
}
