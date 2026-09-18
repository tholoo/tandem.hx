;; Tandem's editor boundary. No agent runtime and no synthetic keystrokes.
;; API target: mattwparas/helix 09d67dfe7300ab18c267e6b0cbfbb493cce21d37.
(require "helix/editor.scm")
(require "helix/ext.scm")
(require "helix/misc.scm")
(require "helix/keymaps.scm")
(require (prefix-in config. "helix/configuration.scm"))
(require-builtin helix/core/keymaps as keys.)
(require "rich.scm")
(require (prefix-in cmd. "helix/commands.scm"))
(require-builtin helix/core/static as editor.)
(require-builtin helix/core/editor as core.)
(require-builtin helix/core/text as text.)
(require-builtin helix/components as ui.)
(require-builtin steel/process)
(require-builtin steel/json)
(require-builtin steel/meta)
(require-builtin steel/filesystem)
(require "steel/result")
(require "steel/sync")
(require (prefix-in chat. "chat.scm"))
(require (prefix-in loc. "location.scm"))
(require (prefix-in compose. "composer.scm"))
(require (prefix-in compare. "comparison.scm"))
(require (prefix-in overlay. "overlay.scm"))

(provide tandem tandem-stop tandem-hide tandem-next tandem-previous tandem-apply tandem-revert tandem-close tandem-peek tandem-return tandem-stops tandem-compose tandem-send tandem-compose-close)

(define (install-keybindings bindings)
  (keys.keymap-update-documentation! bindings
    (hash "tandem" "Open Tandem conversation"
      "tandem-stop"
      "Stop Tandem session"
      "tandem-hide"
      "Hide conversation"
      "tandem-next"
      "Next tour stop or change"
      "tandem-previous"
      "Previous tour stop or change"
      "tandem-apply"
      "Apply proposal"
      "tandem-revert"
      "Revert current change"
      "tandem-close"
      "Close tour"
      "tandem-peek"
      "Compare saved change"
      "tandem-return"
      "Return to tour stop"
      "tandem-stops"
      "Search tour stops"
      "tandem-compose"
      "Edit message"
      "tandem-send"
      "Send message"
      "tandem-compose-close"
      "Keep draft and return to chat"))
  (config.keybindings bindings))

(define connection (box #f))
(define output (box #f))
(define view (box #f))
(define serial (box 0))
(define generation (box -1))
(define card-visible (box #f))
(define card-area (box #f))
(define tour-origin (box #f))
(define peek-request (box #f))
(define peek-origin (box #f))
(define saved-navigation-keys (box #f))
;; Tandem's Previous/Next follows tours or review changes. Restore the user's
;; original bracket motions (normally classes) when the session disconnects.
(define (navigation-keymap previous next)
  (hash "normal" (hash "[" (hash "t" previous) "]" (hash "t" next))))
(define (navigation-keys enabled)
  (cond [(and enabled (not (unbox saved-navigation-keys)))
         ;; The query API cannot round-trip custom command arguments/sequences.
         ;; Respect custom mappings instead of capturing them incompletely.
         (when (and (equal? (query-global-keymap "normal" '("[" "t")) "goto_prev_class")
                (equal? (query-global-keymap "normal" '("]" "t")) "goto_next_class"))
           (set-box! saved-navigation-keys (navigation-keymap "goto_prev_class" "goto_next_class"))
           (add-global-keybinding (navigation-keymap ":tandem-previous" ":tandem-next"))
           (install-keybindings (config.get-keybindings)))]
    [(and (not enabled) (unbox saved-navigation-keys))
      (add-global-keybinding (unbox saved-navigation-keys))
      (set-box! saved-navigation-keys #f)]))
(define navigating (box #f))
(define connection-id (box 0))
(define project-root (box #f))
(define shadow-root (box #f))
(define (field object key fallback)
  (if (and (hash? object) (hash-contains? object key)) (hash-ref object key) fallback))
(define (filter-map f xs) (filter (lambda (x) x) (map f xs)))
(define (has-editor-view?)
  ;; Focus can point at the root container after the last view is removed.
  (not (null? (filter-map editor-doc-in-view? (editor-all-documents)))))
(define (active-tour) (field (unbox view) 'tour #f))
(define (stop-end stop) (inexact->exact (hash-ref stop 'end_line)))
(define (stop-location stop)
  (let* ([start (inexact->exact (hash-ref stop 'line))]
         [end (stop-end stop)]
         [count (+ 1 (- end start))])
    (string-append (hash-ref stop 'file) ":" (number->string start)
      (if (> count 1) (string-append "–" (number->string end)) "")
      " · "
      (number->string count)
      (if (= count 1) " line" " lines"))))

(define (send action)
  (when (unbox output)
    (set-box! serial (+ 1 (unbox serial)))
    (write-string (string-append (value->jsexpr-string
                                  (hash-insert (hash-insert action 'version 1) 'id (unbox serial)))
                   "\n")
      (unbox output))
    (flush-output-port (unbox output))))

(define (publish-context)
  ;; q! removes the final view before post-command/save callbacks finish.
  ;; Looking up its document at that point panics inside Helix's view tree.
  (when (and (unbox output) (not (unbox navigating)) (has-editor-view?))
    (let* ([doc (editor->doc-id (editor-focus))]
           [rope (editor->text doc)]
           [line (editor.get-current-line-number)]
           [start (text.rope-line->char rope (max 0 (- line 8)))]
           [end (min (text.rope-len-chars rope) (+ start 4000))]
           [path (editor-document->path doc)]
           [selection (editor.selection->primary-range (editor.current-selection-object))]
           [from (editor.range->from selection)]
           [to (editor.range->to selection)]
           [selected (or (> (- to from) 1) (equal? (editor-mode) (string->editor-mode "select")))]
           [dirty (filter-map (lambda (id)
                               (if (and (not (compose.composer-doc? id)) (not (compare.comparison-doc? id)) (editor-document-dirty? id))
                                 (or (editor-document->path id) "[unsaved buffer]")
                                 #f))
                   (editor-all-documents))])
      (unless (or (compose.composer-doc? doc) (compare.comparison-doc? doc))
        (let ([context (hash 'file (if path path void)
                        'line
                        (+ line 1)
                        'column
                        (+ 1 (editor.get-current-column-number))
                        'selection
                        (editor.current-highlighted-text!)
                        'selection_start_line
                        (if selected (+ 1 (text.rope-char->line rope from)) void)
                        'selection_end_line
                        (if selected (+ 1 (text.rope-char->line rope (max from (- to 1)))) void)
                        'nearby
                        (text.rope->string (text.rope->slice rope start end))
                        'dirty
                        dirty)])
          (chat.chat-context! context)
          (send (hash 'action "context" 'context context)))))))

(define (user-action action)
  (let ([text (field action 'text "")] [kind (field action 'action "")])
    (cond [(or (equal? kind "compose") (equal? text "/compose")) (tandem-compose)]
      [(equal? text "/peek") (tandem-peek)]
      [(equal? text "/return") (tandem-return)]
      [(equal? text "/stops") (tandem-stops)]
      [else (close-comparison) (publish-context) (send action)])))
(define (close-comparison)
  (set-box! peek-request #f)
  (set-box! navigating #t)
  (compare.close-comparison)
  (set-box! navigating #f))
(define (tandem-peek)
  (cond [(compare.comparison-active?) (close-comparison) (publish-context)]
    [(unbox peek-request) (set-box! peek-request #f)]
    [(compose.composer-active?) (set-error! "Return to a code buffer before comparing.")]
    [(unbox output)
      (publish-context)
      (set-box! peek-origin (loc.bookmark))
      (set-box! peek-request (+ 1 (unbox serial)))
      (send (hash 'action "peek"))]))
(define (tandem-return)
  (let ([tour (active-tour)])
    (if (hash? tour) (user-action (hash 'action "tour_jump" 'index (inexact->exact (hash-ref tour 'current_stop))))
      (set-error! "No active tour."))))
(define (tandem-stops)
  (let ([tour (active-tour)])
    (if (hash? tour)
      (let ([id (hash-ref tour 'id)] [stops (hash-ref tour 'stops)])
        (overlay.pick "Tour stops"
          (cons (cons "0  Overview" 0)
            (map (lambda (index)
                  (let ([stop (list-ref stops index)])
                    (cons (string-append (number->string (+ index 1)) "  " (hash-ref stop 'title)
                           " · "
                           (stop-location stop))
                      (+ index 1))))
              (range 0 (length stops))))
          (lambda (index)
            (if (equal? id (field (active-tour) 'id #f))
              (user-action (hash 'action "tour_jump" 'index index))
              (set-error! "Tour changed; reopen the stop list.")))))
      (set-error! "No active tour."))))
(define (tandem-compose)
  (close-comparison)
  (publish-context)
  (chat.chat-blur)
  (set-box! navigating #t)
  (compose.open-composer (if (equal? (chat.chat-draft) "/compose") "" (chat.chat-draft)) chat.chat-draft!)
  (set-box! navigating #f))
(define (tandem-compose-close)
  (when (compose.composer-active?)
    (chat.chat-draft! (compose.composer-text))
    (set-box! navigating #t)
    (compose.close-composer)
    (set-box! navigating #f)
    (publish-context)
    (chat.chat-focus)))
(define (tandem-send)
  (if (compose.composer-active?)
    (if (field (unbox view) 'busy #f)
      (set-status! "Agent is working; your message draft is kept.")
      (begin (tandem-compose-close) (chat.chat-submit)))
    (set-error! "Open the message editor with Space t e first.")))
(define (tandem-next)
  (user-action (hash 'action (if (hash? (active-tour)) "tour_next" "next_change"))))
(define (tandem-previous)
  (user-action (hash 'action (if (hash? (active-tour)) "tour_previous" "previous_change"))))
(define (tandem-apply) (user-action (hash 'action "apply")))
(define (tandem-revert)
  (user-action (hash 'action "revert" 'change (inexact->exact (field (unbox view) 'change_index 0)))))
(define (tandem-close) (user-action (hash 'action "tour_close")))

(define card-scroll (box 0))
(define card-stop (box #f))
(define card-rows (box 0))
(define (render-card state area frame)
  (let ([tour (active-tour)])
    (when (hash? tour)
      (let* ([index (inexact->exact (hash-ref tour 'current_stop))]
             [stops (hash-ref tour 'stops)]
             [stop (if (= index 0) #f (list-ref stops (- index 1)))]
             [x (ui.area-x area)]
             [y (ui.area-y area)]
             [available (max 1 (- (ui.area-width area) (chat.chat-width)))]
             [width (max 1 (- available 2))]
             [title (if stop (hash-ref stop 'title) (hash-ref tour 'title))]
             [body (if stop (hash-ref stop 'body) (hash-ref tour 'overview))]
             [source (if (equal? (field (field tour 'source #f) 'kind "") "repository") "Repository" "Preview")]
             [location (if stop (string-append "`" (stop-location stop) "`") "Overview")]
             [lines (rich-lines body width)]
             [height (min (max 5 (quotient (ui.area-height area) 3))
                      (max 5 (min 8 (+ 4 (length lines)))))]
             [room (max 1 (- height 4))]
             [identity (list (hash-ref tour 'id) index body)]
             [style (ui.theme-scope-ref "ui.text")]
             [heading (string-append source " · " (number->string index) " / " (number->string (length stops)) "  " title)])
        (unless (equal? identity (unbox card-stop))
          (set-box! card-stop identity)
          (set-box! card-scroll 0))
        (set-box! card-scroll (min (unbox card-scroll) (max 0 (- (length lines) room))))
        (set-box! card-rows (max 0 (- (length lines) room)))
        (set-box! card-area (ui.area x y available height))
        (set-editor-clip-top! height)
        (for-each (lambda (row) (ui.frame-set-string! frame x (+ y row) (make-string available #\space) style)) (range 0 height))
        (ui.frame-set-string! frame (+ x 1) y (substring heading 0 (min width (string-length heading)))
          (ui.style-with-bold (ui.theme-scope-ref "function")))
        (rich-draw frame (+ x 1) (+ y 1) (car (rich-lines location width)))
        (for-each (lambda (row)
                   (rich-draw frame (+ x 1) (+ y row 2) (list-ref lines (+ row (unbox card-scroll)))))
          (range 0 (min room (- (length lines) (unbox card-scroll)))))
        (let ([hint (string-append "`[t` prev · `]t` next · "
                     (if (equal? (field (field tour 'source #f) 'kind "") "repository") "/close" "/apply"))])
          (rich-draw frame (+ x 1) (+ y height -2) (car (rich-lines hint width))))
        (ui.frame-set-string! frame x (+ y height -1) (make-string available #\─)
          (ui.theme-scope-ref "ui.linenr"))))))
(define (card-event state event)
  (cond
    [(and (unbox card-area) (ui.mouse-event? event)
        (ui.mouse-event-within-area? event (unbox card-area)))
      (cond [(= (ui.event-mouse-kind event) 10)
             (set-box! card-scroll (min (unbox card-rows) (+ 2 (unbox card-scroll))))]
        [(= (ui.event-mouse-kind event) 11)
          (set-box! card-scroll (max 0 (- (unbox card-scroll) 2)))])
      ui.event-result/consume]
    [else ui.event-result/ignore]))

(define (align-destination)
  (let ([bounds (active-reading-range)])
    (if (and bounds (> (cadr bounds) (car bounds)))
      (editor.align_view_top)
      (editor.align_view_center))))
(define (center-destination)
  (when (has-editor-view?)
    (let ([doc (editor->doc-id (editor-focus))]
          [line (editor.get-current-line-number)]
          [revision (unbox generation)])
      (align-destination)
      ;; Clipping takes effect in the next editor render. Recenter once after
      ;; that layout pass, only if the destination is still current.
      (enqueue-thread-local-callback-with-delay 40
        (lambda ()
          (when (and (has-editor-view?) (= revision (unbox generation))
                 (equal? doc (editor->doc-id (editor-focus)))
                 (= line (editor.get-current-line-number)))
            (align-destination)))))))

(define (active-reading-range)
  (let* ([tour (active-tour)] [index (field tour 'current_stop 0)])
    (and (hash? tour) (> index 0)
      (let ([stop (list-ref (hash-ref tour 'stops) (- (inexact->exact index) 1))])
        (list (inexact->exact (hash-ref stop 'line)) (stop-end stop))))))

(define (highlight-reading-range)
  (let ([bounds (active-reading-range)])
    (when bounds
      (let* ([rope (editor->text (editor->doc-id (editor-focus)))]
             [count (text.rope-len-lines rope)]
             [first (max 0 (min (- (car bounds) 1) (- count 1)))]
             [last (min (cadr bounds) count)]
             [start (text.rope-line->char rope first)]
             [end (if (< last count) (text.rope-line->char rope last) (text.rope-len-chars rope))])
        (editor.normal_mode)
        (editor.set-current-selection-object! (editor.range->selection (editor.range end start)))))))

(define (receive-live message)
  (if (and (unbox peek-request) (number? (field message 'id #f)) (= (hash-ref message 'id) (unbox peek-request)))
    (begin
      (set-box! peek-request #f)
      (when (hash? (field message 'comparison #f))
        (let ([mark (unbox peek-origin)])
          (if (and (equal? (hash-ref mark 'doc) (editor->doc-id (editor-focus)))
               (= (hash-ref mark 'line) (+ 1 (editor.get-current-line-number)))
               (not (editor-document-dirty? (hash-ref mark 'doc))))
            (begin
              (set-box! navigating #t)
              (compare.open-comparison (hash-ref message 'comparison))
              (set-box! navigating #f))
            (set-status! "Code moved or changed; press Space t p again to compare."))))
      (unless (or (hash? (field message 'comparison #f)) (string? (field message 'error #f)))
        (let ([error "Comparison unavailable from this controller. Finish this session, then run :tandem-stop in Helix and start a new one with :tandem."])
          (set-error! error)
          (chat.chat-receive (hash 'event "error" 'text error))))
      (chat.chat-receive (hash-remove message 'text)))
    (chat.chat-receive message))
  (when (equal? (field message 'event "") "error")
    (set-error! (field message 'text "Tandem error")))
  (when (equal? (field message 'event "") "connected")
    (set-box! project-root (field message 'project #f))
    (navigation-keys #t)
    (set-status! "Tandem connected. Ask for a repository tour in the conversation panel.")
    (publish-context))
  (let ([next (field message 'view #f)] [error (field message 'error #f)])
    (when (string? error) (set-error! error))
    (when (hash? next)
      (let* ([before (active-tour)] [after (field next 'tour #f)]
             [repository? (lambda (tour) (equal? (field (field tour 'source #f) 'kind "") "repository"))])
        (when (and (repository? after) (not (repository? before)))
          (set-box! tour-origin (if (compose.composer-active?) (compose.composer-origin) (loc.bookmark))))
        (when (and (repository? before) (not (hash? after)) (equal? (field next 'stage "") "discuss"))
          (set-box! navigating #t)
          (loc.restore (unbox tour-origin))
          (set-box! navigating #f)
          (set-box! tour-origin #f)
          (publish-context)))
      (set-box! view next)
      (set-box! project-root (field next 'real #f))
      (set-box! shadow-root (field next 'shadow #f))
      (let ([tour (active-tour)])
        (cond [(and (hash? tour) (not (unbox card-visible)))
               (set-box! card-visible #t)
               (push-component! (ui.new-component! "tandem-tour" void render-card (hash "handle_event" card-event)))]
          [(and (not (hash? tour)) (unbox card-visible))
            (set-box! card-visible #f)
            (set-box! card-area #f)
            (set-editor-clip-top! 0)
            (pop-last-component-by-name! "tandem-tour")]))
      (when (and (not (compose.composer-active?)) (not (= (field next 'generation 0) (unbox generation))))
        (close-comparison)
        (set-box! generation (field next 'generation 0))
        (set-box! navigating #t)
        ;; External apply/revert never reloads over unsaved edits.
        (when (or (equal? (field next 'stage "") "applied")
               (equal? (field next 'stage "") "tour"))
          ;; The document-reload binding passes every window to each document,
          ;; including windows that never displayed it. Use Helix's native
          ;; :reload command in a view initialized for that document instead.
          (let ([origin (editor->doc-id (editor-focus))])
            (for-each (lambda (doc)
                       (when (and (editor-document->path doc) (path-exists? (editor-document->path doc)) (not (editor-document-dirty? doc)))
                         (core.editor-switch-action! doc (Action/Replace))
                         (cmd.reload)))
              (editor-all-documents))
            (core.editor-switch-action! origin (Action/Replace))))
        (let ([location (field next 'navigation #f)])
          (when (hash? location)
            (cmd.open (hash-ref location 'file))
            (let* ([bounds (active-reading-range)]
                   [line (if bounds (car bounds) (inexact->exact (hash-ref location 'line)))])
              (cmd.goto-line (max 1 (min line
                                     (text.rope-len-lines (editor->text (editor->doc-id (editor-focus))))))))
            (highlight-reading-range)
            (center-destination)))
        (set-box! navigating #f)
        (publish-context)
        void))))

(define (receive message)
  (when (has-editor-view?) (receive-live message)))

(define (disconnect id)
  (when (= id (unbox connection-id))
    (set-box! connection #f)
    (set-box! output #f)
    (set-box! view #f)
    (set-box! generation -1)
    (overlay.close-overlay)
    (close-comparison)
    (set-box! peek-request #f)
    (chat.chat-close)
    (navigation-keys #f)
    (when (unbox card-visible)
      (set-box! card-visible #f)
      (set-box! card-area #f)
      (set-editor-clip-top! 0)
      (pop-last-component-by-name! "tandem-tour"))))

(define (connect-process args)
  (unless (unbox connection)
    (set-box! connection-id (+ 1 (unbox connection-id)))
    (let* ([id (unbox connection-id)]
           [process (unwrap-ok (spawn-process
                                (with-stdin-piped (with-stdout-piped
                                                   (command "tandem" args)))))]
           [input (child-stdout process)])
      (set-box! connection process)
      (set-box! output (child-stdin process))
      (spawn-native-thread
        (lambda ()
          (let loop ()
            (let ([line (read-line input)])
              (if (eof-object? line)
                (begin (wait process) (hx.with-context (lambda () (disconnect id))))
                (begin
                  (let ([message (string->jsexpr line)])
                    (hx.with-context (lambda ()
                                      (when (= id (unbox connection-id)) (receive message)))))
                  (loop)))))))
      void)))

;; When a tour currently shows a shadow file, reconnect to its real checkout.
(define (current-project)
  (let ([path (or (editor-document->path (editor->doc-id (editor-focus))) ".")]
        [shadow (unbox shadow-root)])
    (if (and shadow (> (string-length path) (string-length shadow))
         (equal? (substring path 0 (+ 1 (string-length shadow))) (string-append shadow "/")))
      (unbox project-root)
      path)))
(define (tandem)
  (close-comparison)
  (chat.chat-open user-action)
  (if (unbox connection)
    (chat.chat-focus)
    (connect-process (list "editor" (current-project)))))
(define (tandem-stop)
  (if (unbox connection)
    (begin
      (user-action (hash 'action "stop"))
      ;; Stop closes the transport before its EOF callback reaches the editor.
      ;; Suppress context hooks during that interval; EOF handles UI cleanup.
      (set-box! output #f))
    (set-error! "No Tandem session is connected.")))
(define (tandem-hide) (chat.chat-close))
(register-hook 'post-insert-char (lambda (_) (publish-context)))
(register-hook 'post-command (lambda (_) (publish-context)))
(register-hook 'selection-did-change (lambda (_) (publish-context)))
;; This Helix version emits document-saved when the asynchronous save starts.
;; Refresh after completion; bound retries if saving fails or editing continues.
(define (refresh-after-save doc attempts)
  (enqueue-thread-local-callback-with-delay 100
    (lambda ()
      (publish-context)
      (when (and (> attempts 0) (member doc (editor-all-documents))
             (editor-document-dirty? doc))
        (refresh-after-save doc (- attempts 1))))))
(register-hook 'document-saved (lambda (doc) (refresh-after-save doc 49)))
(register-hook 'terminal-focus-lost publish-context)

;; Merge existing keys over defaults, preserving custom commands and sequences.
(let ([defaults (keys.helix-string->keymap (value->jsexpr-string
                                            (hash "normal" (hash "space" (hash "t"
                                                                          (hash "p" ":tandem-peek" "r" ":tandem-return" "j" ":tandem-stops"
                                                                            "e"
                                                                            ":tandem-compose"
                                                                            "s"
                                                                            ":tandem-send"
                                                                            "q"
                                                                            ":tandem-compose-close"))))))])
  (keys.helix-merge-keybindings defaults (config.get-keybindings))
  (install-keybindings defaults))
