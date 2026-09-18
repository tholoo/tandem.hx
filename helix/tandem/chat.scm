;; Native conversation UI. The controller owns commands, stages, and agent work.
(require "helix/misc.scm")
(require "helix/editor.scm")
(require "rich.scm")
(require-builtin helix/components as ui.)
(provide chat-state chat-restore chat-open chat-close chat-focus chat-blur chat-receive chat-width chat-context! chat-draft chat-draft! chat-submit)

(define visible (box #f))
(define focused (box #f))
(define bounds (box #f))
(define reserved (box 0))
(define history (box (list "Ask a question or request a repository tour. Discuss an approach, then /begin to implement.")))
(define draft (box ""))
(define caret (box 0))
(define recalled-messages (box #f))
(define recall-index (box 0))
(define scroll (box 0))
(define session-view (box #f))
(define dispatch (box #f))
(define context (box #f))
(define completion-index (box 0))
(define completion-dismissed (box #f))
(define commands
  (list (list "/begin" "Implement discussion" 'begin)
        (list "/apply" "Apply saved preview" 'apply)
        (list "/cancel" "Interrupt agent" 'cancel)
        (list "/peek" "Compare this change" 'proposal)
        (list "/return" "Return to tour stop" 'tour)
        (list "/stops" "Find a tour stop" 'tour)
        (list "/compose" "Expand message editor" 'always)
        (list "/next" "Next stop or change" 'navigation)
        (list "/prev" "Previous stop or change" 'navigation)
        (list "/jump" "Jump to stop number" 'tour)
        (list "/close" "Close repository tour" 'repository)
        (list "/diff" "Show proposal diff" 'proposal)
        (list "/revert" "Undo applied change" 'applied)
        (list "/proposal" "Choose proposal number" 'proposal)))
(define (starts? value prefix)
  (and (>= (string-length value) (string-length prefix))
       (equal? (substring value 0 (string-length prefix)) prefix)))
(define (disabled item)
  (let* ([v (unbox session-view)] [stage (field v 'stage "")]
         [kind (list-ref item 2)] [tour (field v 'tour #f)]
         [dirty (field (unbox context) 'dirty (list))])
    (cond [(and (member kind '(begin apply applied)) (not (null? dirty))) "save code buffers first"]
          [(and (busy?) (member kind '(begin apply proposal applied))) "agent is working"]
          [(and (equal? kind 'begin) (equal? stage "tour")) "request refinements in chat"]
          [(and (equal? kind 'apply) (not (equal? stage "tour"))) "no pending preview"]
          [(and (equal? kind 'cancel) (not (busy?))) "agent is idle"]
          [(and (equal? kind 'proposal) (not (number? (field v 'proposal #f)))) "no proposal yet"]
          [(and (equal? kind 'tour) (not (hash? tour))) "no active tour"]
          [(and (equal? kind 'repository)
                (not (equal? (field (field tour 'source #f) 'kind "") "repository"))) "no repository tour"]
          [(and (equal? kind 'navigation) (not (hash? tour)) (not (equal? stage "applied"))) "no stops or changes"]
          [(and (equal? kind 'applied) (not (equal? stage "applied"))) "apply a proposal first"]
          [else #f])))
(define (completions)
  (if (and (unbox focused) (not (unbox completion-dismissed))
           (starts? (unbox draft) "/") (not (member #\space (string->list (unbox draft)))))
      (filter (lambda (item) (starts? (car item) (unbox draft))) commands) (list)))
(define (complete-command)
  (let ([items (completions)])
    (unless (null? items)
      (let ([item (list-ref items (modulo (unbox completion-index) (length items)))])
        (chat-draft! (string-append (car item) (if (member (car item) '("/jump" "/proposal")) " " "")))
        (set-box! completion-dismissed #t)))))
(define (chat-context! value) (set-box! context value))
(define (chat-draft) (unbox draft))
(define (chat-draft! value)
  (set-box! draft value) (set-box! caret (string-length value))
  (set-box! completion-index 0) (set-box! completion-dismissed #f))
(define (recall-message direction)
  (when (and (> direction 0) (not (unbox recalled-messages)))
    (let ([messages (reverse (filter (lambda (text) (starts? text "You: ")) (unbox history)))])
      (unless (null? messages)
        (set-box! recalled-messages
          (cons (box (cons (unbox draft) (unbox caret)))
                (map (lambda (text)
                       (let ([message (substring text 5)])
                         (box (cons message (string-length message))))) messages)))
        (set-box! recall-index 0))))
  (when (unbox recalled-messages)
    (let* ([messages (unbox recalled-messages)]
           [index (max 0 (min (+ (unbox recall-index) direction) (- (length messages) 1)))])
      (set-box! (list-ref messages (unbox recall-index)) (cons (unbox draft) (unbox caret)))
      (let ([entry (unbox (list-ref messages index))])
        (set-box! draft (car entry)) (set-box! caret (cdr entry)))
      (set-box! recall-index index)
      ;; Recalled slash commands stay in history navigation until edited.
      (set-box! completion-index 0) (set-box! completion-dismissed #t)
      (when (= index 0) (set-box! recalled-messages #f)))))
(define (chat-blur) (set-box! focused #f))
(define (context-label)
  (let* ([c (unbox context)] [file (field c 'file #f)]
         [root (field (unbox session-view) 'real "")]
         [shadow (field (unbox session-view) 'shadow "")]
         [relative (if (string? file)
           (cond [(starts? file (string-append shadow "/")) (substring file (+ 1 (string-length shadow)))]
                 [(starts? file (string-append root "/")) (substring file (+ 1 (string-length root)))]
                 [else file]) "no code file")]
         [first-value (field c 'selection_start_line #f)] [last-value (field c 'selection_end_line #f)]
         [first (if (number? first-value) first-value #f)] [last (if (number? last-value) last-value #f)])
    (string-append (if (and (string? file) (starts? file (string-append shadow "/"))) "Context (preview): " "Context: ") relative
      (if (string? file) (string-append ":" (number->string (or first (field c 'line 1)))
           (if (and first last (not (= first last))) (string-append "–" (number->string last)) "")) "")
      (if first " · selection" "")
      (if (member file (field c 'dirty (list))) " · unsaved" ""))))
(define activity (box ""))
(define pulse (box 0))
(define ticking (box #f))
(define spinner (list "⠋" "⠙" "⠹" "⠸" "⠼" "⠴" "⠦" "⠧" "⠇" "⠏"))
(define (field object key fallback)
  (if (and (hash? object) (hash-contains? object key)) (hash-ref object key) fallback))
(define (chat-width) (unbox reserved))
(define (busy?) (field (unbox session-view) 'busy #f))
;; Only schedule redraws while visible and working; idle panels have no timer.
(define (animate)
  (when (and (unbox visible) (busy?) (not (unbox ticking)))
    (set-box! ticking #t)
    (enqueue-thread-local-callback-with-delay 120
      (lambda ()
        (set-box! ticking #f)
        (when (and (unbox visible) (busy?))
          (set-box! pulse (modulo (+ 1 (unbox pulse)) (length spinner))))
        (animate)))))
(define (clean text)
  (list->string (filter (lambda (c) (or (char=? c #\newline) (char>=? c #\space))) (string->list text))))
(define (append-history text)
  (let ([items (append (unbox history) (list (clean text)))])
    (set-box! history (drop items (max 0 (- (length items) 200))))))
(define (chat-receive message)
  (let ([event (field message 'event "")]
        [next (field message 'view #f)]
        [error (field message 'error #f)]
        [text (field message 'text #f)])
    (when (hash? next)
      (when (and (field next 'busy #f) (not (busy?))) (set-box! activity ""))
      (set-box! session-view next)
      (animate))
    (cond [(equal? event "history")
           (set-box! recalled-messages #f)
           (let ([items (field message 'lines (list))])
             (set-box! history (if (null? items)
               (list "Ask a question or request a repository tour. Discuss an approach, then /begin to implement.")
               (map clean items))))]
          [(equal? event "message") (append-history text)]
          [(equal? event "activity") (set-box! activity (clean text))]
          [(equal? event "error") (append-history (string-append "Error: " text))]
          [(and (equal? event "") (string? text)) (append-history text)])
    (when (string? error) (append-history (string-append "Error: " error)))))
(define (write-at frame x y width text style)
  (ui.frame-set-string! frame x y (substring text 0 (min width (string-length text))) style))
;; Component removal is queued; a hidden panel can still get one render call.
(define (render state area frame)
  (when (unbox visible) (render-chat state area frame)))
(define (render-chat state area frame)
  (let* ([width (min 75 (max 1 (- (ui.area-width area) 20)) (max 35 (quotient (* (ui.area-width area) 5) 12)))]
         [x (+ (ui.area-x area) (- (ui.area-width area) width))]
         [y (ui.area-y area)]
         [height (ui.area-height area)]
         [inner (max 1 (- width 3))]
         [style (ui.theme-scope-ref "ui.text")]
         [menu (completions)]
         [menu-rows (min 5 (length menu) (max 0 (- height 9)))]
         [heading (ui.style-with-bold (ui.theme-scope-ref "function"))])
    (set-box! reserved width)
    (set-box! bounds (ui.area x y width height))
    (set-editor-clip-right! width)
    (for-each (lambda (row)
      (ui.frame-set-string! frame x (+ y row) (make-string width #\space) style)
      (ui.frame-set-string! frame x (+ y row) "│" style)) (range 0 height))
    (when (> height 0)
      (write-at frame (+ x 2) y inner
        (string-append "Tandem · " (field (unbox session-view) 'stage "connecting")
                       (if (busy?) (string-append " · " (list-ref spinner (unbox pulse))) " · ready")) heading))
    (when (> height (+ 6 menu-rows))
      (let* ([all (apply append (map (lambda (item)
                     (append (rich-lines item inner) (list (list)))) (unbox history)))]
             [room (- height 6 menu-rows)]
             [offset (min (unbox scroll) (max 0 (- (length all) room)))]
             [end (- (length all) offset)]
             [start (max 0 (- end room))])
        (set-box! scroll offset)
        (for-each (lambda (index)
          (let ([line (list-ref all index)])
            (rich-draw frame (+ x 2) (+ y 1 (- index start)) line))) (range start end))))
    (when (> height 4)
      (write-at frame (+ x 2) (+ y height -4) inner (context-label) (ui.theme-scope-ref "comment")))
    (when (> menu-rows 0)
      (let* ([chosen (modulo (unbox completion-index) (length menu))]
             [first (max 0 (- chosen (- menu-rows 1)))])
        (for-each (lambda (row)
          (let* ([index (+ first row)] [item (list-ref menu index)] [reason (disabled item)])
            (write-at frame (+ x 2) (+ y height -4 (- menu-rows) row) inner
              (string-append (if (= index chosen) "> " "  ") (car item) "  " (or reason (cadr item)))
              (ui.theme-scope-ref (if (= index chosen) "ui.menu.selected" (if reason "comment" "function"))))))
          (range 0 menu-rows))))
    (when (> height 3)
      (write-at frame (+ x 2) (+ y height -3) inner
        (if (busy?)
            (string-append (list-ref spinner (unbox pulse)) " Working "
              (list->string (map (lambda (c) (if (char=? c #\newline) #\space c)) (string->list (unbox activity)))))
            (cond [(equal? (field (unbox session-view) 'stage "") "discuss") "Ready · /begin to build"]
                  [(equal? (field (unbox session-view) 'stage "") "tour") "Preview · /apply when ready"]
                  [(equal? (field (unbox session-view) 'stage "") "applied") "Applied · edit or discuss next change"]
                  [else "Ready"]))
        (ui.theme-scope-ref (if (busy?) "warning" "ui.text.info"))))
    (when (> height 2)
      (write-at frame (+ x 2) (+ y height -2) inner
        (if (unbox focused)
            (if (busy?) "↑↓ history · Ctrl+C cancel · Esc code" (if (null? menu) "↑↓ history · Enter send · Esc code" "↑/↓ choose · Tab complete · Enter send"))
            "Space a / :tandem to chat") (ui.theme-scope-ref "ui.text.info"))
      (let* ([offset (max 0 (- (unbox caret) (- inner 3)))]
             [tail (list->string (map (lambda (c) (if (char=? c #\newline) #\↵ c)) (string->list (substring (unbox draft) offset))))])
        (write-at frame (+ x 2) (+ y height -1) inner (string-append "> " tail) style)))))
(define (cursor state area)
  (if (and (unbox focused) (unbox bounds))
      (let* ([area (unbox bounds)] [inner (max 1 (- (ui.area-width area) 3))]
             [offset (max 0 (- (unbox caret) (- inner 3)))])
        (list (ui.position (+ (ui.area-y area) (ui.area-height area) -1)
                           (min (+ (ui.area-x area) (ui.area-width area) -1)
                                (+ (ui.area-x area) 4 (- (unbox caret) offset)))) 'bar))
      #f))
(define (insert text)
  (set-box! completion-index 0) (set-box! completion-dismissed #f)
  (let ([text (list->string (map (lambda (c) (if (char=? c #\newline) #\space c)) (string->list (clean text))))])
    (set-box! draft (string-append (substring (unbox draft) 0 (unbox caret)) text (substring (unbox draft) (unbox caret))))
    (set-box! caret (+ (unbox caret) (string-length text)))))
(define (command-draft?)
  (let loop ([chars (string->list (unbox draft))])
    (cond [(null? chars) #f]
          [(char=? (car chars) #\space) (loop (cdr chars))]
          [else (char=? (car chars) #\/)])))
(define (submit)
  (when (> (string-length (unbox draft)) 0)
    ;; Unavailable commands explain why without destroying the draft.
    (define exact (filter (lambda (item) (equal? (car item) (unbox draft))) commands))
    ;; Slash commands (including /cancel and navigation) still reach the controller.
    (cond [(and (not (null? exact)) (disabled (car exact))) (set-status! (disabled (car exact))) #f]
          [(and (busy?) (not (command-draft?)))
        (set-status! "Agent is working. Ctrl+C cancels; your draft is kept.") #f]
          [else (begin
          ((unbox dispatch) (hash 'action "input" 'text (unbox draft)))
          (set-box! recalled-messages #f)
          (chat-draft! "")
          (set-box! scroll 0) #t)])))
(define (chat-submit) (submit))
(define (handle state event)
  (cond
    [(ui.mouse-event? event)
     (if (and (unbox bounds) (ui.mouse-event-within-area? event (unbox bounds)))
         (begin
           (cond [(= (ui.event-mouse-kind event) 0)
                  (set-box! focused #t)]
                 [(= (ui.event-mouse-kind event) 11) (set-box! scroll (+ 3 (unbox scroll)))]
                 [(= (ui.event-mouse-kind event) 10) (set-box! scroll (max 0 (- (unbox scroll) 3)))])
           ui.event-result/consume)
         (begin (when (= (ui.event-mouse-kind event) 0) (set-box! focused #f)) ui.event-result/ignore))]
    [(not (unbox focused)) ui.event-result/ignore]
    [(ui.paste-event? event) (insert (ui.paste-event-string event)) ui.event-result/consume]
    [(ui.key-event? event)
     (let ([character (ui.key-event-char event)] [modifier (ui.key-event-modifier event)])
       (cond [(ui.key-event-escape? event) (set-box! focused #f)]
             [(ui.key-event-enter? event) (submit)]
             [(ui.key-event-tab? event) (complete-command)]
             [(and (ui.key-event-down? event) (not (null? (completions)))) (set-box! completion-index (+ 1 (unbox completion-index)))]
             [(and (ui.key-event-up? event) (not (null? (completions))))
              (set-box! completion-index (modulo (- (unbox completion-index) 1) (length (completions))))]
             [(ui.key-event-up? event) (recall-message 1)]
             [(ui.key-event-down? event) (recall-message -1)]
             [(ui.key-event-page-up? event) (set-box! scroll (+ 10 (unbox scroll)))]
             [(ui.key-event-page-down? event) (set-box! scroll (max 0 (- (unbox scroll) 10)))]
             [(ui.key-event-left? event) (set-box! caret (max 0 (- (unbox caret) 1)))]
             [(ui.key-event-right? event) (set-box! caret (min (string-length (unbox draft)) (+ (unbox caret) 1)))]
             [(ui.key-event-home? event) (set-box! caret 0)]
             [(ui.key-event-end? event) (set-box! caret (string-length (unbox draft)))]
             [(ui.key-event-backspace? event)
              (set-box! completion-index 0) (set-box! completion-dismissed #f)
              (when (> (unbox caret) 0)
                (set-box! draft (string-append (substring (unbox draft) 0 (- (unbox caret) 1)) (substring (unbox draft) (unbox caret))))
                (set-box! caret (- (unbox caret) 1)))]
             [(ui.key-event-delete? event)
              (when (< (unbox caret) (string-length (unbox draft)))
                (set-box! draft (string-append (substring (unbox draft) 0 (unbox caret)) (substring (unbox draft) (+ 1 (unbox caret))))))]
             [(and character (= modifier ui.key-modifier-ctrl))
              (cond [(char=? character #\x) ((unbox dispatch) (hash 'action "compose"))]
                    [(char=? character #\c) ((unbox dispatch) (hash 'action "cancel"))]
                    [(char=? character #\a) (set-box! caret 0)]
                    [(char=? character #\e) (set-box! caret (string-length (unbox draft)))]
                    [(char=? character #\u) (set-box! draft "") (set-box! caret 0)])]
             [(and character (or (= modifier 0) (= modifier ui.key-modifier-shift))) (insert (string character))]))
     ui.event-result/consume]
    [else ui.event-result/ignore]))
(define (chat-open send-action)
  (set-box! dispatch send-action)
  (unless (unbox visible)
    (set-box! visible #t)
    (push-component! (ui.new-component! "tandem-chat" void render (hash "handle_event" handle "cursor" cursor))))
  (set-box! focused #t)
  (animate))
(define (chat-focus) (chat-open (unbox dispatch)))
(define (chat-close)
  (when (unbox visible)
    (set-box! visible #f)
    (set-box! focused #f)
    (set-box! reserved 0)
    (set-box! bounds #f)
    (set-editor-clip-right! 0)
    (pop-last-component-by-name! "tandem-chat")))

(define (chat-state) (list (unbox visible) (unbox focused)))
(define (chat-restore state)
  (when (car state)
    (chat-focus)
    (unless (cadr state) (chat-blur))))
