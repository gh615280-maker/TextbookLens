import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { useParams, useSearchParams } from 'react-router-dom';

import { useLanguage } from '../../app/LanguageProvider';
import type { DocumentLocator } from '../../lib/generated/document';
import type { ProviderProfileSummary } from '../../lib/generated/provider';
import type { AppSettingsDto } from '../../lib/generated/settings';
import { TauriLearningApi } from '../learning/api';
import { useLearningRequestSnapshot } from '../learning/LearningRequestProvider';
import {
  ConversationPanelOwner,
  conversationPanels,
} from '../history/conversation-api';
import { NoteEditor } from '../notes/NoteEditor';
import { TauriNotesApi, type Note } from '../notes/api';
import { RegionSelectionOverlay } from '../learning/RegionSelectionOverlay';
import { SelectionMenu } from '../learning/SelectionMenu';
import {
  deferredLearningSurfacePort,
  learningProfileForRegion,
  menuSnapshotFromRegion,
  menuSnapshotFromText,
  releaseSnapshotCapture,
  type LearningProfile,
  type LearningSelectionSnapshot,
} from '../learning/selection-state';
import type { ReaderBootstrap, ReaderSection, ReaderSettings } from './api';
import { TauriReaderApi } from './api';
import { DocxReaderAdapter } from './docx/DocxReaderAdapter';
import { EpubReaderAdapter } from './epub/EpubReaderAdapter';
import { MarkerLayer } from './markers/MarkerLayer';
import type { MarkerHistoryLabels } from './markers/MarkerLayer';
import { OverlappingMarkerMenu } from './markers/OverlappingMarkerMenu';
import type { AnnotationMarker } from './contracts';
import { PdfReaderAdapter } from './pdf/PdfReaderAdapter';
import { ReaderController } from './ReaderController';
import { ReaderLayout } from './ReaderLayout';
import {
  sectionIdForRegionSelection,
  sectionIdForTextSelection,
} from './section-resolution';

export function ReaderPage() {
  const { bookId } = useParams();
  const [searchParams] = useSearchParams();
  const { message, uiLanguage } = useLanguage();
  const api = useMemo(() => new TauriReaderApi(), []);
  const learningApi = useMemo(() => new TauriLearningApi(), []);
  const learningRequests = useLearningRequestSnapshot();
  const noteApi = useMemo(() => new TauriNotesApi(), []);
  const [settings, setSettings] = useState<ReaderSettings | null>(null);
  const [bootstrap, setBootstrap] = useState<ReaderBootstrap | null>(null);
  const [sections, setSections] = useState<ReaderSection[]>([]);
  const [panelContent, setPanelContent] = useState<string>();
  const [currentLocator, setCurrentLocator] = useState<DocumentLocator | null>(
    null,
  );
  const [firstHintVisible, setFirstHintVisible] = useState(false);
  const [learningProfile, setLearningProfile] =
    useState<LearningProfile | null>(null);
  const [visionLearningProfile, setVisionLearningProfile] =
    useState<LearningProfile | null>(null);
  const [learningSelection, setLearningSelection] =
    useState<Readonly<LearningSelectionSnapshot> | null>(null);
  const [regionSelecting, setRegionSelecting] = useState(false);
  const [editingNote, setEditingNote] = useState<Readonly<Note> | null>(null);
  const [overlappingMarkers, setOverlappingMarkers] = useState<{
    readonly markers: readonly AnnotationMarker[];
    readonly returnFocus: HTMLElement | null;
  } | null>(null);
  const readerContainerRef = useRef<HTMLDivElement>(null);
  const markerHistoryRef = useRef<HTMLDivElement>(null);
  const controllerRef = useRef<ReaderController>(null);
  const markerLayerRef = useRef<MarkerLayer>(null);
  const conversationOwnerRef = useRef<ConversationPanelOwner>(null);
  const hintCompletionInFlight = useRef(false);
  const learningProfileRef = useRef<LearningProfile | null>(null);
  const refreshedMarkerRequestsRef = useRef(new Set<string>());
  const sectionsRef = useRef<ReaderSection[]>([]);
  const messageRef = useRef(message);
  const uiLanguageRef = useRef(uiLanguage);
  const noteLabels = {
    input: message('notes.input'),
    save: message('notes.save'),
    cancel: message('notes.cancel'),
    empty: message('notes.empty'),
    tooLong: message('notes.tooLong'),
    error: message('notes.error'),
    conflict: message('notes.conflict'),
    reload: message('notes.reload'),
    preserve: message('notes.preserve'),
    delete: message('notes.delete'),
    deleteConfirm: message('notes.deleteConfirm'),
    confirmDelete: message('notes.confirmDelete'),
    cancelDelete: message('notes.cancelDelete'),
  };

  useEffect(() => {
    learningProfileRef.current = learningProfile;
  }, [learningProfile]);
  useEffect(() => {
    sectionsRef.current = sections;
  }, [sections]);
  useEffect(() => {
    messageRef.current = message;
  }, [message]);
  useEffect(() => {
    uiLanguageRef.current = uiLanguage;
    markerLayerRef.current?.setLabels(markerHistoryLabels(uiLanguage));
  }, [uiLanguage]);
  useEffect(() => {
    let shouldRefresh = false;
    for (const request of learningRequests.requests) {
      if (
        request.status !== 'completed' ||
        !request.conversationId ||
        request.presentation?.bookId ||
        refreshedMarkerRequestsRef.current.has(request.requestId)
      )
        continue;
      refreshedMarkerRequestsRef.current.add(request.requestId);
      shouldRefresh = true;
    }
    if (shouldRefresh) {
      const controller = controllerRef.current;
      void controller?.refreshAnnotations().then(() => {
        if (controllerRef.current !== controller) return;
        const relocations = controller.getMarkerRelocations();
        if (
          relocations.length > 0 &&
          relocations.every(
            (relocation) => relocation.relocationStatus !== 'unresolved',
          )
        ) {
          setPanelContent(undefined);
        }
      });
    }
  }, [learningRequests]);
  const activateMarker = useCallback(
    (marker: AnnotationMarker) => {
      if (!bookId) return;
      if (marker.kind === 'note') {
        void noteApi
          .get(bookId, marker.id)
          .then(setEditingNote)
          .catch(() => setPanelContent(messageRef.current('notes.error')));
        return;
      }
      const owner = conversationOwnerRef.current;
      if (!marker.conversationId || !owner) {
        setPanelContent('Unable to open this conversation.');
        return;
      }
      void conversationPanels
        .open(bookId, marker.conversationId, marker.id, owner)
        .catch(() => setPanelContent('Unable to load this conversation.'));
    },
    [bookId, noteApi],
  );
  useEffect(() => {
    if (!bookId) return;
    return conversationPanels.onDeleted((deletedBookId) => {
      if (deletedBookId === bookId)
        void controllerRef.current?.refreshAnnotations();
    });
  }, [bookId]);

  const completeFirstHint = useCallback(() => {
    if (hintCompletionInFlight.current) return;
    hintCompletionInFlight.current = true;
    void invoke<AppSettingsDto>('complete_first_reader_hint')
      .then((next) => setFirstHintVisible(!next.firstReaderHintCompleted))
      .catch(() => {})
      .finally(() => {
        hintCompletionInFlight.current = false;
      });
  }, []);

  useEffect(() => {
    void api
      .getReaderSettings()
      .then(setSettings)
      .catch(() => {});
    void invoke<AppSettingsDto>('get_app_settings')
      .then(async (value) => {
        setFirstHintVisible(!value.firstReaderHintCompleted);
        const [learningProfiles, visionProfiles] = await Promise.all([
          invoke<ProviderProfileSummary[]>('list_provider_profiles', {
            operation: 'text_learning',
          }),
          invoke<ProviderProfileSummary[]>('list_provider_profiles', {
            operation: 'vision_learning',
          }),
        ]);
        const selected = learningProfiles.find(
          (profile) =>
            profile.id ===
            (value.defaultLearningProfileId ?? value.activeProviderProfileId),
        );
        const selectedVision = visionProfiles.find(
          (profile) => profile.id === value.defaultVisionProfileId,
        );
        setLearningProfile(
          selected ? { id: selected.id, modelId: selected.modelId } : null,
        );
        setVisionLearningProfile(
          selectedVision
            ? { id: selectedVision.id, modelId: selectedVision.modelId }
            : null,
        );
      })
      .catch(() => {});
    if (!bookId) return;
    void api
      .getReaderBootstrap(bookId)
      .then((value) => {
        setBootstrap(value);
        setCurrentLocator(value.lastLocator);
        return api.listReaderSections(bookId);
      })
      .then(setSections)
      .catch(() => {
        setBootstrap(null);
        setSections([]);
      });
  }, [api, bookId]);
  useEffect(() => {
    const container = readerContainerRef.current;
    if (!bookId || !container) return;
    const owner = new ConversationPanelOwner(bookId);
    conversationOwnerRef.current = owner;
    const markerLayer = new MarkerLayer(
      markerHistoryRef.current,
      (markers) => {
        if (markers.length === 1) {
          activateMarker(markers[0]);
          return;
        }
        setOverlappingMarkers({
          markers,
          returnFocus:
            document.activeElement instanceof HTMLElement
              ? document.activeElement
              : null,
        });
      },
      async (marker, summaryText) => {
        if (!marker.revision) throw new Error('ANNOTATION_REVISION_MISSING');
        await api.updateAiAnnotationSummary(
          bookId,
          marker.id,
          marker.revision,
          summaryText,
        );
        marker.revision += 1;
      },
      markerHistoryLabels(uiLanguageRef.current),
    );
    markerLayerRef.current = markerLayer;
    const controller = new ReaderController(
      api,
      {
        pdf: (events) => new PdfReaderAdapter(container, events),
        epub: (events) => new EpubReaderAdapter(container, events),
        docx: (events) => new DocxReaderAdapter(container, events),
      },
      {
        onSelection: (selection) => {
          if (!selection) return;
          setPanelContent(undefined);
          completeFirstHint();
          const profile = learningProfileRef.current;
          const sectionId = sectionIdForTextSelection(
            selection,
            sectionsRef.current,
          );
          if (!profile || !sectionId) return;
          setLearningSelection(
            menuSnapshotFromText(selection, {
              bookId,
              sectionId,
              profile,
              position: selectionPosition(),
            }),
          );
        },
        onProgress: (progress) => setCurrentLocator(progress.locator),
        onMarkerActivate: (markers) => {
          if (markers.length === 1) {
            activateMarker(markers[0]);
            return;
          }
          setOverlappingMarkers({
            markers,
            returnFocus:
              document.activeElement instanceof HTMLElement
                ? document.activeElement
                : null,
          });
        },
        onMarkersResolved: () => setPanelContent(undefined),
        onFailure: (error) => setPanelContent(error.message),
      },
      markerLayer,
    );
    controllerRef.current = controller;
    void controller
      .open(bookId)
      .then(() => api.listReaderSections(bookId))
      .then((value) => {
        if (controllerRef.current === controller) setSections(value);
      })
      .catch(() => {});
    return () => {
      owner.dispose();
      if (conversationOwnerRef.current === owner)
        conversationOwnerRef.current = null;
      controller.dispose();
      if (markerLayerRef.current === markerLayer) markerLayerRef.current = null;
      setOverlappingMarkers(null);
      setLearningSelection((current) => {
        releaseSnapshotCapture(current);
        return null;
      });
      if (controllerRef.current === controller) controllerRef.current = null;
    };
  }, [activateMarker, api, bookId, completeFirstHint]);
  const updateSettings = (next: ReaderSettings) => {
    void api
      .updateReaderSettings(next)
      .then(setSettings)
      .catch(() => {});
  };
  const beginRegionSelection = useCallback(() => {
    if (
      !bookId ||
      (!learningProfile && !visionLearningProfile) ||
      !sections.length ||
      regionSelecting
    ) {
      setPanelContent(message('learning.unavailable'));
      return;
    }
    setPanelContent(undefined);
    setRegionSelecting(true);
    void controllerRef.current
      ?.beginRegionSelection()
      .then((region) => {
        if (!region) return;
        const sectionId = sectionIdForRegionSelection(region, sections);
        const profile = learningProfileForRegion(
          region,
          learningProfile,
          visionLearningProfile,
        );
        if (!profile || !sectionId) {
          region.capture?.release();
          setPanelContent(message('learning.unavailable'));
          return;
        }
        setLearningSelection(
          menuSnapshotFromRegion(region, {
            bookId,
            sectionId,
            profile,
            position: regionSelectionPosition(),
          }),
        );
      })
      .finally(() => setRegionSelecting(false));
  }, [
    bookId,
    learningProfile,
    message,
    regionSelecting,
    sections,
    visionLearningProfile,
  ]);
  return (
    <>
      {searchParams.get('index') === 'local-only' ? (
        <p aria-live="polite" role="status">
          {message('indexStart.localLimitation')}
        </p>
      ) : null}
      <ReaderLayout
        title={bootstrap?.book.title ?? (bookId ? '阅读教材' : '阅读器')}
        language={uiLanguage}
        location={formatLocation(currentLocator, uiLanguage)}
        firstHintVisible={firstHintVisible}
        onCompleteFirstHint={completeFirstHint}
        bookId={bookId ?? null}
        format={bootstrap?.book.format ?? 'pdf'}
        sections={sections}
        settings={settings ?? undefined}
        panelContent={panelContent}
        search={api.searchBook.bind(api)}
        onSettingsChange={updateSettings}
        onNavigate={(locator) =>
          controllerRef.current?.navigate(locator) ?? Promise.resolve(false)
        }
        readerContainerRef={readerContainerRef}
        markerHistoryRef={markerHistoryRef}
        onStartRegionSelection={
          (learningProfile || visionLearningProfile) && sections[0]
            ? beginRegionSelection
            : undefined
        }
        regionSelecting={regionSelecting}
      />
      <RegionSelectionOverlay
        active={regionSelecting}
        instruction={message('learning.region.instruction')}
        status={message('learning.region.status')}
        onCancel={() => {
          controllerRef.current?.cancelRegionSelection();
          setRegionSelecting(false);
        }}
      />
      {learningSelection && (
        <SelectionMenu
          api={learningApi}
          noteApi={noteApi}
          labels={{
            menu: message('learning.menu'),
            explain: message('learning.explain'),
            example: message('learning.example'),
            derive: message('learning.derive'),
            translate: message('learning.translate'),
            ask: message('learning.ask'),
            note: message('learning.note'),
            input: message('learning.input'),
            submit: message('learning.submit'),
            unavailable: message('learning.unavailable'),
            error: message('learning.error'),
            noteEditor: noteLabels,
            confirmation: {
              title: message('learning.confirm.title'),
              details: message('learning.confirm.details', {
                provider: '{provider}',
                profile: '{profile}',
                model: '{model}',
                tokens: '{tokens}',
                sources: '{sources}',
                citations: '{citations}',
              }),
              noPrompt: message('learning.confirm.noPrompt'),
              cancel: message('learning.confirm.cancel'),
              continue: message('learning.confirm.continue'),
              imageRisk: message('learning.confirm.imageRisk'),
              costRisk: message('learning.confirm.costRisk'),
            },
          }}
          snapshot={learningSelection}
          surface={deferredLearningSurfacePort}
          onClose={() => setLearningSelection(null)}
          onError={setPanelContent}
          onNoteSaved={() => {
            void controllerRef.current?.refreshAnnotations();
          }}
        />
      )}
      {editingNote ? (
        <div className="local-note-editor-surface">
          <NoteEditor
            api={noteApi}
            anchor={editingNote.anchor}
            bookId={editingNote.bookId}
            labels={noteLabels}
            note={editingNote}
            sectionId={editingNote.sectionId}
            selectedText={editingNote.selectedText}
            onCancel={() => setEditingNote(null)}
            onDeleted={() => {
              setEditingNote(null);
              void controllerRef.current?.refreshAnnotations();
            }}
            onSaved={() => {
              setEditingNote(null);
              void controllerRef.current?.refreshAnnotations();
            }}
          />
        </div>
      ) : null}
      {overlappingMarkers ? (
        <OverlappingMarkerMenu
          markers={overlappingMarkers.markers}
          returnFocus={overlappingMarkers.returnFocus}
          onActivate={(marker) => {
            setOverlappingMarkers(null);
            activateMarker(marker);
          }}
          onClose={() => setOverlappingMarkers(null)}
        />
      ) : null}
    </>
  );
}

function markerHistoryLabels(
  language: 'zh-CN' | 'zh-TW' | 'en',
): MarkerHistoryLabels {
  if (language === 'en') {
    return {
      locate: (sequence) => `Locate question ${sequence} in the textbook`,
      open: 'Open answer',
      edit: 'Edit description',
      save: 'Save',
      cancel: 'Cancel',
      description: (sequence) => `Description for question ${sequence}`,
      unresolved: 'The original location could not be restored precisely.',
      saveFailed: 'The description could not be saved.',
    };
  }
  if (language === 'zh-TW') {
    return {
      locate: (sequence) => `定位問題 ${sequence} 的原文位置`,
      open: '查看回答',
      edit: '編輯簡述',
      save: '儲存',
      cancel: '取消',
      description: (sequence) => `問題 ${sequence} 的簡短說明`,
      unresolved: '原文位置無法精確恢復。',
      saveFailed: '簡述儲存失敗。',
    };
  }
  return {
    locate: (sequence) => `定位问题 ${sequence} 的原文位置`,
    open: '查看回答',
    edit: '编辑简述',
    save: '保存',
    cancel: '取消',
    description: (sequence) => `问题 ${sequence} 的简短说明`,
    unresolved: '原文位置无法精确恢复。',
    saveFailed: '简述保存失败。',
  };
}

function selectionPosition() {
  const range = document.getSelection()?.rangeCount
    ? document.getSelection()?.getRangeAt(0)
    : null;
  const rect = range?.getBoundingClientRect();
  return { x: rect?.left ?? 8, y: rect?.bottom ?? 8 };
}

function regionSelectionPosition() {
  return {
    x: Math.max(8, window.innerWidth / 2 - 160),
    y: Math.max(96, window.innerHeight / 3),
  };
}

function formatLocation(
  locator: DocumentLocator | null,
  language: 'zh-CN' | 'zh-TW' | 'en',
): string | null {
  if (!locator) return null;
  if (locator.format === 'pdf')
    return language === 'en'
      ? `Page ${locator.startPage}`
      : language === 'zh-TW'
        ? `第 ${locator.startPage} 頁`
        : `第 ${locator.startPage} 页`;
  if (locator.format === 'epub')
    return language === 'en'
      ? 'Current section'
      : language === 'zh-TW'
        ? '目前章節'
        : '当前章节';
  return language === 'en'
    ? 'Current block'
    : language === 'zh-TW'
      ? '目前段落'
      : '当前段落';
}
