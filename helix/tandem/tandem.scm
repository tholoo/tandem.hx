;; Tandem's editor boundary. No agent runtime and no synthetic keystrokes.
;; API target: mattwparas/helix 09d67dfe7300ab18c267e6b0cbfbb493cce21d37.
(require "helix/editor.scm")
(require "helix/ext.scm")
(require "helix/misc.scm")
(require (prefix-in cmd. "helix/commands.scm"))
(require-builtin helix/core/static as editor.)
(require-builtin helix/core/text as text.)
(require-builtin helix/components as ui.)
(require-builtin steel/process)
(require-builtin steel/json)
(require-builtin steel/meta)
(require-builtin steel/filesystem)
(require "steel/result")
(require "steel/sync")

(provide tandem-connect tandem-next tandem-previous tandem-review tandem-revert tandem-close)

(define connection (box #f))
(define output (box #f))
(define view (box #f))
(define serial (box 0))
(define generation (box -1))
(define card-visible (box #f))
(define card-area (box #f))
(define navigating (box #f))
(define (field object key fallback)
  (if (and (hash? object) (hash-contains? object key)) (hash-ref object key) fallback))
(define (filter-map f xs) (filter (lambda (x) x) (map f xs)))
(define (active-tour) (field (unbox view) 'tour #f))

(define (send action)
  (when (unbox output)
    (set-box! serial (+ 1 (unbox serial)))
    (write-string (string-append (value->jsexpr-string
                (hash-insert (hash-insert action 'version 1) 'id (unbox serial))) "\n")
               (unbox output))
    (flush-output-port (unbox output))))

(define (publish-context)
  (when (and (unbox output) (not (unbox navigating)))
    (let* ([doc (editor->doc-id (editor-focus))]
           [rope (editor->text doc)]
           [line (editor.get-current-line-number)]
           [start (text.rope-line->char rope (max 0 (- line 8)))]
           [end (min (text.rope-len-chars rope) (+ start 4000))]
           [path (editor-document->path doc)]
           [dirty (filter-map (lambda (id)
                                (if (editor-document-dirty? id)
                                    (or (editor-document->path id) "[unsaved buffer]") #f))
                              (editor-all-documents))])
      (send (hash 'action "context" 'context
                  (hash 'file (if path path void)
                        'line (+ line 1) 'column (+ 1 (editor.get-current-column-number))
                        'selection (editor.current-highlighted-text!)
                        'nearby (text.rope->string (text.rope->slice rope start end))
                        'dirty dirty))))))

(define (user-action action)
  (publish-context)
  (send action))
(define (tandem-next)
  (user-action (hash 'action (if (hash? (active-tour)) "tour_next" "next_change"))))
(define (tandem-previous)
  (user-action (hash 'action (if (hash? (active-tour)) "tour_previous" "previous_change"))))
(define (tandem-review) (user-action (hash 'action "apply")))
(define (tandem-revert)
  (user-action (hash 'action "revert" 'change (inexact->exact (field (unbox view) 'change_index 0)))))
(define (tandem-close) (user-action (hash 'action "tour_close")))

;; Wrap narration locally; source remains the main editor area.
(define (wrap-line line width)
  (if (<= (string-length line) width) (list line)
      (cons (substring line 0 width) (wrap-line (substring line width) width))))
(define (wrapped string width)
  (apply append (map (lambda (line) (wrap-line line width)) (split-many string "\n"))))
(define (render-card state area frame)
  (let ([tour (active-tour)])
    (when (hash? tour)
      (let* ([index (inexact->exact (hash-ref tour 'current_stop))]
             [stops (hash-ref tour 'stops)]
             [stop (if (= index 0) #f (list-ref stops (- index 1)))]
             [height (min (max 3 (- (ui.area-height area) 3)) (if stop 8 12))]
             [x (ui.area-x area)]
             [y (+ (ui.area-y area) (- (ui.area-height area) height))]
             [width (max 1 (- (ui.area-width area) 2))]
             [title (if stop (hash-ref stop 'title) (hash-ref tour 'title))]
             [body (if stop (hash-ref stop 'body) (hash-ref tour 'overview))]
             [style (ui.theme-scope-ref "ui.text")]
             [heading (string-append (number->string index) " / " (number->string (length stops)) "  " title)])
        (set-box! card-area (ui.area x y (ui.area-width area) height))
        (set-editor-clip-bottom! height)
        (for-each (lambda (row) (ui.frame-set-string! frame x (+ y row) (make-string (ui.area-width area) #\space) style)) (range 0 height))
        (ui.frame-set-string! frame (+ x 1) y (substring heading 0 (min width (string-length heading))) (ui.style-with-bold style))
        (let loop ([lines (wrapped body width)] [row 2])
          (when (and (not (null? lines)) (< row (- height 1)))
            (ui.frame-set-string! frame (+ x 1) (+ y row) (car lines) style)
            (loop (cdr lines) (+ row 1))))
        (let ([buttons (string-append "[Previous]  [Next]  "
                         (if (equal? (field (field tour 'source #f) 'kind "") "repository") "[Close]" "[Review]"))])
          (ui.frame-set-string! frame (+ x 1) (+ y height -1)
            (substring buttons 0 (min width (string-length buttons))) style))))))
(define (card-event state event)
  (if (and (unbox card-area) (ui.mouse-event? event)
           (= (ui.event-mouse-kind event) 0)
           (ui.mouse-event-within-area? event (unbox card-area))
           (= (ui.event-mouse-row event) (+ (ui.area-y (unbox card-area)) (ui.area-height (unbox card-area)) -1)))
      (begin
        (let ([col (- (ui.event-mouse-col event) (ui.area-x (unbox card-area)))])
          (cond [(< col 12) (tandem-previous)] [(< col 20) (tandem-next)]
                [(equal? (field (field (active-tour) 'source #f) 'kind "") "repository") (tandem-close)]
                [else (tandem-review)]))
        (ui.event-result/consume))
      (ui.event-result/ignore)))

(define (receive message)
  (let ([next (field message 'view #f)] [error (field message 'error #f)])
    (when (string? error) (set-error! error))
    (when (hash? next)
      (set-box! view next)
      (let ([tour (active-tour)])
        (cond [(and (hash? tour) (not (unbox card-visible)))
               (set-box! card-visible #t)
               (push-component! (ui.new-component! "tandem-tour" void render-card (hash "handle_event" card-event)))]
              [(and (not (hash? tour)) (unbox card-visible))
               (set-box! card-visible #f) (set-box! card-area #f)
               (set-editor-clip-bottom! 0) (pop-last-component-by-name! "tandem-tour")]))
      (when (not (= (field next 'generation 0) (unbox generation)))
        (set-box! generation (field next 'generation 0))
        (set-box! navigating #t)
        ;; External apply/revert never reloads over unsaved edits.
        (when (or (equal? (field next 'stage "") "review")
                  (equal? (field next 'stage "") "tour"))
          (for-each (lambda (doc)
                      (when (and (editor-document->path doc) (path-exists? (editor-document->path doc)) (not (editor-document-dirty? doc)))
                        (editor-document-reload doc)))
                    (editor-all-documents)))
        (let ([location (field next 'navigation #f)])
          (when (hash? location)
            (cmd.open (hash-ref location 'file))
            (cmd.goto-line (max 1 (min (inexact->exact (hash-ref location 'line))
                             (text.rope-len-lines (editor->text (editor->doc-id (editor-focus)))))))))
        (set-box! navigating #f)
        void))))

(define (tandem-connect)
  (unless (unbox connection)
    (let* ([socket (env-var "TANDEM_SOCKET")]
           [process (unwrap-ok (spawn-process
                      (with-stdin-piped (with-stdout-piped
                        (command "tandem" (list "bridge" socket))))))]
           [input (child-stdout process)])
      (set-box! connection process)
      (set-box! output (child-stdin process))
      (spawn-native-thread
        (lambda ()
          (let loop ()
            (let ([line (read-line input)])
              (unless (eof-object? line)
                (let ([message (string->jsexpr line)])
                  (hx.with-context (lambda () (receive message))))
                (loop))))))
      void)))

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
