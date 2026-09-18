;; Temporary keyboard-only tour picker. Never switches the code document.
(require "helix/misc.scm")
(require (prefix-in chat. "chat.scm"))
(require-builtin helix/components as ui.)
(provide pick close-overlay)
(define title (box ""))
(define options (box #f))
(define query (box ""))
(define selected (box 0))
(define offset (box 0))
(define choose (box #f))
(define (close-overlay) (pop-last-component-by-name! "tandem-overlay"))
(define (matches)
  (if (unbox options)
      (if (equal? (unbox query) "") (unbox options)
          (map (lambda (label) (car (filter (lambda (option) (equal? label (car option))) (unbox options))))
               (fuzzy-match (unbox query) (map car (unbox options)))))
      (list)))
(define (write frame x y width text style)
  (ui.frame-set-string! frame x y (substring text 0 (min width (string-length text))) style))
(define (draw-picker area frame)
  (let* ([available (max 1 (- (ui.area-width area) (chat.chat-width)))]
         [width (max 4 (min 86 (- available 4)))]
         [height (max 7 (min 18 (- (ui.area-height area) 4) (+ 6 (length (unbox options)))))]
         [x (+ (ui.area-x area) (max 0 (quotient (- available width) 2)))]
         [y (+ (ui.area-y area) (max 0 (quotient (- (ui.area-height area) height) 3)))]
         [inner (- width 2)] [room (- height 6)] [items (matches)]
         [base (ui.theme-scope-ref "ui.popup")]
         [border (ui.theme-scope-ref "ui.window")]
         [muted (ui.theme-scope-ref "ui.text.info")]
         [focus (ui.theme-scope-ref "ui.menu.selected")])
    (set-box! selected (min (unbox selected) (max 0 (- (length items) 1))))
    (set-box! offset (max 0 (- (unbox selected) (- room 1))))
    (for-each (lambda (row)
      (write frame x (+ y row) width (make-string width #\space) base)
      (write frame x (+ y row) 1 "│" border)
      (write frame (+ x width -1) (+ y row) 1 "│" border)) (range 0 height))
    (write frame x y width (string-append "╭" (make-string inner #\─) "╮") border)
    (write frame (+ x 2) y (- width 4) (string-append " " (unbox title) " ")
      (ui.style-with-bold (ui.theme-scope-ref "ui.text.focus")))
    (write frame (+ x 2) (+ y 1) (- width 4) (string-append "> " (unbox query) "▏")
      (ui.theme-scope-ref "ui.text"))
    (write frame x (+ y 2) width (string-append "├" (make-string inner #\─) "┤") border)
    (if (null? items)
        (write frame (+ x 3) (+ y 3) (- width 6) "No matching stops" muted)
        (for-each (lambda (row)
          (let* ([index (+ row (unbox offset))]
                 [active (= index (unbox selected))]
                 [label (car (list-ref items index))]
                 [limit (max 1 (- width 6))]
                 [short (if (> (string-length label) limit)
                            (string-append (substring label 0 (- limit 1)) "…") label)]
                 [style (if active focus (ui.theme-scope-ref "ui.text"))])
            (write frame (+ x 1) (+ y row 3) inner (make-string inner #\space) (if active focus base))
            (write frame (+ x 2) (+ y row 3) (- width 4) (string-append (if active "› " "  ") short) style)))
          (range 0 (min room (- (length items) (unbox offset))))))
    (write frame (+ x 2) (+ y height -3) (- width 4)
      (string-append (number->string (length items)) " / " (number->string (length (unbox options))) " stops") muted)
    (write frame (+ x 2) (+ y height -2) (- width 4) "↑↓ / Ctrl-n/p · Enter jump · Esc close" muted)
    (write frame x (+ y height -1) width (string-append "╰" (make-string inner #\─) "╯") border)))
(define (draw state area frame) (draw-picker area frame))
(define (move amount) (set-box! selected (max 0 (+ (unbox selected) amount))))
(define (accept)
  (let ([items (matches)])
    (unless (null? items)
      (let ([value (cdr (list-ref items (min (unbox selected) (- (length items) 1))))] [callback (unbox choose)])
        (close-overlay) (callback value)))))
(define (handle state event)
  (when (ui.key-event? event)
    (cond [(ui.key-event-escape? event) (close-overlay)]
          [(ui.key-event-up? event) (move -1)] [(ui.key-event-down? event) (move 1)]
          [(ui.key-event-page-up? event) (move -10)] [(ui.key-event-page-down? event) (move 10)]
          [(and (unbox options) (ui.key-event-char event) (= (ui.key-event-modifier event) ui.key-modifier-ctrl))
           (cond [(char=? (ui.key-event-char event) #\n) (move 1)]
                 [(char=? (ui.key-event-char event) #\p) (move -1)]
                 [(char=? (ui.key-event-char event) #\u) (set-box! query "") (set-box! selected 0)])]
          [(and (unbox options) (ui.key-event-enter? event)) (accept)]
          [(and (unbox options) (ui.key-event-backspace? event))
           (set-box! query (substring (unbox query) 0 (max 0 (- (string-length (unbox query)) 1)))) (set-box! selected 0)]
          [(and (unbox options) (ui.key-event-char event) (= (ui.key-event-modifier event) 0))
           (set-box! query (string-append (unbox query) (string (ui.key-event-char event)))) (set-box! selected 0)]))
  ui.event-result/consume)
(define (open title-text)
  (close-overlay)
  (set-box! title title-text) (set-box! selected 0) (set-box! offset 0) (set-box! query "")
  (push-component! (ui.new-component! "tandem-overlay" void draw (hash "handle_event" handle))))
(define (pick title-text items callback)
  (set-box! options items) (set-box! choose callback) (open title-text))
